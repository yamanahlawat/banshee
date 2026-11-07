import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ready from './mocks/ready.json';
import permissions from './mocks/permissions.json';
import remote from './mocks/remote.json';
import { dictated } from './lib/history.mock';

// Local noon, because the history helpers read the local calendar day.
const NOW = new Date(2026, 7, 27, 12, 0, 0);

function stamp(minutesAgo: number): string {
  return `${new Date(NOW.getTime() - minutesAgo * 60_000).toISOString().slice(0, 19)}Z`;
}

const asked = (id: number, row: Partial<HistoryRow>): HistoryRow => ({
  id,
  kind: 'question',
  question: 'Which branch should I use?',
  text: null,
  outcome: 'no_answer',
  reason: null,
  timestamp: stamp(1),
  audio: null,
  ...row,
});

// The daemon answers oldest first, so the newest row is last.
const rows = [
  dictated(1, 'the older one', stamp(9)),
  dictated(2, 'Yes, open the pull request.', stamp(1)),
];

vi.mock('./lib/tauri', async () => (await import('./lib/tauri.mock')).mockTauri());

import {
  detectAgents,
  history,
  listen,
  listVoices,
  listDevices,
  downloadModels,
  openPermissionPane,
  restartDaemon,
  copyText,
  clearHistory,
  setSetting,
  startDaemon,
  status,
} from './lib/tauri';
import { get } from 'svelte/store';
import type { HistoryRow } from './lib/tauri';
import { agents } from './lib/agents';
import { table as historyTable, view } from './lib/history';
import { daemon, empty, reduceStatus, type Blocker } from './lib/daemon';
import { forgetTheAsk } from './lib/downloads';
import {
  announcement,
  forgetCopy,
  report,
  waitingSays,
  DICTATION_FAILURE,
  WAITING_SAYS,
  WAITS_FOR_ITS_FILE,
} from './lib/copy';
import { forgetKeys } from './lib/keys';
import { PAGES } from './lib/nav';
import { LIST_AND_DETAIL_FROM, SIDEBAR_FOLDS_BELOW } from './lib/layout';
import { stubMatchMedia } from './lib/media.mock';
import App from './App.svelte';

// jsdom draws no modal dialog. This stands in for the parts the window leans
// on: the open state, Escape closing it, and the close event.
function dismissOnEscape(this: HTMLDialogElement, event: KeyboardEvent) {
  if (event.key === 'Escape' && this.dispatchEvent(new Event('cancel', { cancelable: true }))) {
    this.close();
  }
}
HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) {
  this.setAttribute('open', '');
  this.addEventListener('keydown', dismissOnEscape);
};
HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) {
  if (!this.hasAttribute('open')) return;
  this.removeAttribute('open');
  this.removeEventListener('keydown', dismissOnEscape);
  this.dispatchEvent(new Event('close'));
};

// A page's heading is its lead statement, and that sentence moves with the
// daemon. The region keeps the page's name, so tests ask for that and read the
// heading inside it.
const page = (name: string) => screen.getByRole('region', { name });
const pageHeading = (name: string) => within(page(name)).getByRole('heading');
const sidebar = () => screen.getByRole('navigation', { name: 'Banshee' });
// Anchored at the start only: an item's name runs on with its marks.
const item = (name: string) =>
  within(sidebar()).getByRole('button', { name: new RegExp(`^${name}`) });
const choose = (name: string) => fireEvent.click(item(name));
// The newest row reads in the list and again in the detail, so the window has
// read History once its option is drawn.
const listed = () => screen.findByRole('option', { name: /^Yes, open the pull request\./ });

// The daemon's pushes, captured so a test can deliver one.
const pushes = new Map<string, (event: { payload: unknown }) => void>();

beforeEach(async () => {
  await new Promise((resolve) => setTimeout(resolve, 0));
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  vi.clearAllMocks();
  daemon.set(empty());
  agents.set([]);
  historyTable.set({ rows: [], total: 0, loaded: false, saving: null });
  view.set({ query: '', chosen: null });
  forgetCopy();
  forgetTheAsk();
  forgetKeys();
  vi.mocked(status).mockResolvedValue(ready);
  vi.mocked(history).mockResolvedValue(rows);
  // `clearAllMocks` empties the call list and leaves the implementation, so a
  // rejection set by one test is still set for the next one without this.
  vi.mocked(clearHistory).mockResolvedValue(undefined);
  vi.mocked(startDaemon).mockResolvedValue(undefined);
  pushes.clear();
  vi.mocked(listen).mockImplementation((name: string, handler: unknown) => {
    pushes.set(name, handler as (event: { payload: unknown }) => void);
    return Promise.resolve(() => {});
  });
  vi.mocked(listDevices).mockResolvedValue({ devices: [], current: null });
  vi.mocked(listVoices).mockResolvedValue({
    voices: [{ id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true }],
    current: 'af_sky',
  });
  vi.mocked(detectAgents).mockResolvedValue([]);
});

it('draws an empty history rather than leaving the body blank', async () => {
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() => expect(screen.getByText('Nothing here yet')).toBeTruthy());
  expect(screen.queryByRole('listbox')).toBeNull();
});

it('asks for an agent on an empty History only once the daemon says none is connected', async () => {
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Connect an agent' })).toBeTruthy(),
  );
  await fireEvent.click(screen.getByRole('button', { name: 'Connect an agent' }));
  await waitFor(() => expect(pageHeading('Agents')).toBeTruthy());
});

it('asks for no agent while one is connected', async () => {
  vi.mocked(history).mockResolvedValue([]);
  vi.mocked(detectAgents).mockResolvedValue([
    { id: 'claude', name: 'Claude Code', presence: 'connected', note: '' },
  ]);
  render(App);
  await waitFor(() => expect(screen.getByText('Nothing here yet')).toBeTruthy());
  await waitFor(() => expect(get(agents)).toHaveLength(1));
  expect(screen.queryByRole('button', { name: 'Connect an agent' })).toBeNull();
});

it('says what a blocker stops and offers the pane that clears it', async () => {
  vi.mocked(status).mockResolvedValue({
    ...permissions,
    blockers: permissions.blockers as Blocker[],
  });
  render(App);
  const open = await screen.findAllByRole('button', { name: /^Open System Settings for / });
  await fireEvent.click(open[0]);
  expect(vi.mocked(openPermissionPane)).toHaveBeenCalled();
});

it('lands on History when the window is reopened', async () => {
  vi.mocked(status).mockResolvedValue({
    ...permissions,
    blockers: permissions.blockers as Blocker[],
  });
  render(App);
  await screen.findAllByRole('button', { name: /^Open System Settings for / });
  await choose('Listening');
  await waitFor(() => expect(page('Listening')).toBeTruthy());

  pushes.get('app:reopened')?.({ payload: null });

  await waitFor(() => expect(screen.queryByRole('region', { name: 'Listening' })).toBeNull());
  expect(page('History')).toBeTruthy();
  expect(item('History').getAttribute('aria-current')).toBe('page');
  expect(
    screen.getAllByRole('button', { name: /^Open System Settings for / }).length,
  ).toBeGreaterThan(0);
});

it('shows that an agent is waiting, without claiming to know the question', async () => {
  render(App);
  await listed();
  daemon.update((s) => ({ ...s, live: { ...s.live, armed: true } }));
  await waitFor(() =>
    expect(
      screen.getByText('An agent asked a question and is waiting for your answer.'),
    ).toBeTruthy(),
  );
});

it('swaps History for a page, and back again from the sidebar', async () => {
  render(App);
  await listed();

  await choose('Hotkey');
  await waitFor(() => expect(page('Hotkey')).toBeTruthy());
  expect(screen.queryByText('Yes, open the pull request.')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Done' })).toBeNull();

  await choose('History');
  await listed();
});

it('lists every page in the sidebar, with the settings in a named group', async () => {
  render(App);
  await listed();

  const names = within(sidebar())
    .getAllByRole('button')
    .map((button) => button.textContent?.trim());
  expect(names).toEqual([
    'History',
    'Agents',
    'Listening',
    'Speaking',
    'Hotkey',
    'Feedback',
    'Saved history',
  ]);
  const settings = within(sidebar()).getByRole('group', { name: 'Settings' });
  expect(
    within(settings)
      .getAllByRole('button')
      .map((button) => button.textContent?.trim()),
  ).toEqual(['Listening', 'Speaking', 'Hotkey', 'Feedback', 'Saved history']);

  expect(item('History').getAttribute('aria-current')).toBe('page');
  await choose('Feedback');
  await waitFor(() => expect(item('Feedback').getAttribute('aria-current')).toBe('page'));
  expect(item('History').hasAttribute('aria-current')).toBe(false);
});

it('lands focus on the heading of the page chosen, and moves none on open', async () => {
  render(App);
  await listed();
  expect(document.activeElement).toBe(document.body);

  await choose('Hotkey');
  await waitFor(() => expect(document.activeElement).toBe(pageHeading('Hotkey')));

  await choose('History');
  await waitFor(() =>
    expect(document.activeElement).toBe(
      within(page('History')).getByRole('heading', { name: 'History' }),
    ),
  );
});

it('says the state in the sidebar foot on every page', async () => {
  render(App);
  await listed();
  expect(within(sidebar()).getByText('Ready')).toBeTruthy();

  await choose('Agents');
  daemon.update((s) => ({ ...s, live: { ...s.live, activity: 'recording', recording: true } }));
  await waitFor(() => expect(within(sidebar()).getByText('Recording')).toBeTruthy());
  expect(within(sidebar()).queryByText(/Audio goes to/)).toBeNull();
});

it('moves Feedback off the Hotkey page onto its own', async () => {
  render(App);
  await listed();

  await choose('Hotkey');
  await waitFor(() => expect(page('Hotkey')).toBeTruthy());
  expect(screen.queryByRole('radiogroup', { name: 'Feedback' })).toBeNull();

  await choose('Feedback');
  await waitFor(() => expect(page('Feedback')).toBeTruthy());
  expect(within(page('Feedback')).getByRole('radiogroup', { name: 'Feedback' })).toBeTruthy();
});

it('leads Feedback with the mode the daemon reports', async () => {
  // The config still asks for both: the lead reads the cues in force.
  vi.mocked(status).mockResolvedValue({ ...ready, feedback: 'none' });
  render(App);
  await listed();
  await choose('Feedback');
  await waitFor(() => expect(pageHeading('Feedback').textContent).toMatch(/plays no sound\.$/));
});

it('asks before it deletes the record', async () => {
  render(App);
  await listed();
  await choose('Saved history');
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));
  expect(screen.getByText(/This cannot be undone/)).toBeTruthy();
});

it('mounts the whole list where nothing is measurable, with no paging controls', async () => {
  const many = Array.from({ length: 95 }, (_, i) =>
    dictated(i + 1, `dictation ${i + 1}`, stamp(95 - i)),
  );
  vi.mocked(history).mockResolvedValue(many);

  render(App);
  await screen.findByRole('option', { name: /^dictation 95,/ });

  // jsdom lays nothing out, so the windowing math has no viewport to work
  // against and the fallback mounts every row. The windowed path is held by
  // lib/vlist and the list's own tests, where sizes are given.
  expect(screen.getAllByRole('option').length).toBe(95);
  expect(screen.queryByRole('button', { name: /older/i })).toBeNull();
});

it('opens History and its search on Cmd+F, and Escape clears it with the selection kept', async () => {
  render(App);
  await listed();
  await choose('Hotkey');
  await waitFor(() => expect(page('Hotkey')).toBeTruthy());

  await fireEvent.keyDown(window, { key: 'f', metaKey: true });
  const find = await screen.findByRole('searchbox', { name: 'Search' });
  await waitFor(() => expect(document.activeElement).toBe(find));
  expect(page('History')).toBeTruthy();

  await fireEvent.input(find, { target: { value: 'older' } });
  await waitFor(() => expect(screen.getAllByRole('option').length).toBe(1));
  expect(screen.getByRole('option', { name: /^the older one,/ })).toBeTruthy();

  await fireEvent.input(find, { target: { value: 'nothing matches this' } });
  await waitFor(() => expect(screen.getByText('No match')).toBeTruthy());

  await fireEvent.input(find, { target: { value: 'older' } });
  await fireEvent.keyDown(find, { key: 'Escape' });
  await waitFor(() => expect(screen.getAllByRole('option').length).toBe(2));
  expect((find as HTMLInputElement).value).toBe('');
  expect(document.activeElement).toBe(find);
  expect(
    screen.getByRole('option', { name: /^the older one,/ }).getAttribute('aria-selected'),
  ).toBe('true');
});

it('follows the newest row again when the window is reopened', async () => {
  render(App);
  await listed();
  const older = () => screen.getByRole('option', { name: /^the older one,/ });
  await fireEvent.click(older());
  expect(older().getAttribute('aria-selected')).toBe('true');

  vi.mocked(history).mockResolvedValue([...rows, dictated(3, 'The newest one.', stamp(0))]);
  pushes.get('daemon:state')?.({ payload: { history_changes: 1 } });
  const newest = await screen.findByRole('option', { name: /^The newest one\./ });
  expect(newest.getAttribute('aria-selected')).toBe('false');

  pushes.get('app:reopened')?.({ payload: null });
  await waitFor(() =>
    expect(
      screen.getByRole('option', { name: /^The newest one\./ }).getAttribute('aria-selected'),
    ).toBe('true'),
  );
  expect(older().getAttribute('aria-selected')).toBe('false');
});

it('empties the search when the window is reopened', async () => {
  render(App);
  await listed();
  await fireEvent.keyDown(window, { key: 'f', metaKey: true });
  const find = await screen.findByRole('searchbox', { name: 'Search' });
  await fireEvent.input(find, { target: { value: 'older' } });
  await waitFor(() => expect(screen.getAllByRole('option').length).toBe(1));

  pushes.get('app:reopened')?.({ payload: null });
  await waitFor(() => expect(screen.getAllByRole('option').length).toBe(2));
  expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('');
});

// The list below keeps its height only while the stack scrolls on its own.
it('holds tall blockers in a notice region of their own, apart from the list', async () => {
  vi.mocked(status).mockResolvedValue({
    ...permissions,
    blockers: permissions.blockers as Blocker[],
  });
  render(App);
  const fixes = await screen.findAllByRole('button', { name: /^Open System Settings for / });
  const notices = screen.getByRole('region', { name: 'Notices' });
  expect(fixes.every((one) => notices.contains(one))).toBe(true);
  const list = await screen.findByRole('listbox');
  expect(notices.contains(list)).toBe(false);
  expect(list.contains(notices)).toBe(false);
});

// A regex here would throw on the paren.
it('treats find metacharacters as text rather than a pattern', async () => {
  render(App);
  await listed();
  await fireEvent.keyDown(window, { key: 'f', metaKey: true });

  const find = await screen.findByRole('searchbox', { name: 'Search' });
  await fireEvent.input(find, { target: { value: '.*(' } });

  await waitFor(() => expect(screen.getByText('No match')).toBeTruthy());
  expect(screen.queryByRole('option')).toBeNull();
});

it('marks the page that holds a key the daemon has not taken', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['audio.hotkey'] });
  render(App);
  await listed();

  expect(item('Hotkey').textContent).toContain(WAITING_SAYS);
  expect(item('Listening').textContent).not.toContain(WAITING_SAYS);

  // The class carries the dashed rule. jsdom resolves no scoped stylesheet, so
  // the class is the only part of the visual mark a test here can see.
  await choose('Hotkey');
  await waitFor(() =>
    expect(within(page('Hotkey')).getByText(WAITING_SAYS).classList.contains('pending')).toBe(true),
  );
});

it('leaves every page unmarked when the daemon has taken each key', async () => {
  render(App);
  await listed();
  expect(screen.queryByText(WAITING_SAYS)).toBeNull();
});

it('marks the saving switch the daemon has not taken', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['daemon.save_history'] });
  render(App);
  await listed();
  await choose('Saved history');

  const swtch = screen.getByRole('button', {
    name: /Stop saving.*could not use it yet/,
  });
  expect(swtch.classList.contains('pending')).toBe(true);
});

// Every setting applies at once, so a key that waits is one the daemon could
// not use.
it('offers no restart for a key that waits', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['audio.hotkey'] });
  render(App);
  await waitFor(() => expect(screen.getByText(WAITING_SAYS)).toBeTruthy());
  expect(screen.queryByText('Restart to apply')).toBeNull();
});

it('follows what the daemon says about history, not what the config asked for', async () => {
  // The two disagree when the history file will not open: the write landed in
  // the config and the daemon kept its old connection.
  vi.mocked(status).mockResolvedValue({ ...ready, history_enabled: false });
  render(App);
  await listed();
  await choose('Saved history');
  await waitFor(() => expect(screen.getByRole('button', { name: 'Start saving' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Stop saving' })).toBeNull();
});

it('offers a way back when the daemon has stopped', async () => {
  render(App);
  await listed();

  daemon.update((s) => ({ ...s, down: 'not running' }));
  await waitFor(() => expect(screen.getByText('Banshee is not running')).toBeTruthy());
  expect(screen.getByRole('button', { name: 'Start Banshee' })).toBeTruthy();
  // What was said before is still readable.
  expect(await listed()).toBeTruthy();
});

// The box the person sees under the blockers band. It aligns with the bands
// it stands among, not with a text column that no turn is drawing beside it.
it('aligns the stopped-daemon box with the bands above it', async () => {
  render(App);
  await listed();

  daemon.update((s) => ({ ...s, down: 'not running' }));
  await waitFor(() => expect(screen.getByText('Banshee is not running')).toBeTruthy());

  const box = screen.getByText('Banshee is not running').closest('.absence');
  expect(box?.classList.contains('in-record')).toBe(false);
});

it('stops naming a microphone once the daemon has stopped', async () => {
  render(App);
  await listed();

  daemon.update((s) => ({
    ...s,
    live: { ...s.live, audio_device: 'MacBook Pro Microphone', microphone_open: false },
  }));
  await choose('Listening');
  await waitFor(() =>
    expect(pageHeading('Listening').textContent).toBe(
      'Banshee opens the MacBook Pro Microphone when you speak.',
    ),
  );

  daemon.update((s) => ({ ...s, down: 'not running' }));
  await waitFor(() =>
    expect(pageHeading('Listening').textContent).toContain('Banshee is not running'),
  );
});

// The device the words were picked up on is not where they went, and the page
// that says so is the one a person opens to check.
it('says where audio goes when a remote server listens', async () => {
  vi.mocked(status).mockResolvedValue(remote);
  render(App);
  await listed();

  await choose('Listening');
  await waitFor(() =>
    expect(pageHeading('Listening').textContent).toContain(
      'Banshee sends what you say to api.openai.com to be heard.',
    ),
  );
});

// The foot shows on every page, so audio leaving the machine is never one
// click away. The device it is picked up on is on the Listening page.
it('names the host in the sidebar foot on every page, rather than the microphone', async () => {
  vi.mocked(status).mockResolvedValue(remote);
  render(App);
  await listed();

  for (const name of PAGES) {
    await choose(name);
    await waitFor(() => expect(page(name)).toBeTruthy());
    const line = within(sidebar()).getByText(/Audio goes to/);
    expect(line.textContent).toBe('Audio goes to api.openai.com');
    expect(line.textContent).not.toContain('MacBook Pro Microphone');
  }
});

it('names no host in the sidebar while the listening happens on this machine', async () => {
  render(App);
  await listed();

  expect(within(sidebar()).queryByText(/Audio goes to/)).toBeNull();
  expect(item('Listening').textContent).not.toContain(WAITING_SAYS);
});

it('marks Listening for a pending stt.remote key', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['stt.remote.model'] });
  render(App);
  await listed();

  await waitFor(() => expect(item('Listening').textContent).toContain(WAITING_SAYS));
});

it('names a remote listener the daemon cannot name', async () => {
  vi.mocked(status).mockResolvedValue({
    ...remote,
    // The daemon answers with an empty host when the address is no URL, and
    // with null only when the listener is local.
    remote: {
      stt: { remote: true, host: '', key_present: true },
      tts: { remote: false, host: null, speaker_started: true, key_present: false },
    },
  });
  render(App);
  await listed();

  expect(within(sidebar()).getByText(/Audio goes to/).textContent).toBe(
    'Audio goes to a transcription server',
  );

  await choose('Listening');
  await waitFor(() =>
    expect(pageHeading('Listening').textContent).toContain(
      'Banshee sends what you say to a transcription server to be heard.',
    ),
  );
});

it('marks the listener in force when the one asked for could not start', async () => {
  vi.mocked(status).mockResolvedValue({
    ...remote,
    remote: {
      stt: { remote: false, host: null, key_present: true },
      tts: { remote: false, host: null, speaker_started: true, key_present: false },
    },
    pending: ['stt.provider'],
  });
  render(App);
  await listed();

  expect(within(sidebar()).queryByText(/Audio goes to/)).toBeNull();
  expect(item('Listening').textContent).toContain(WAITING_SAYS);
});

it('subscribes to every push the daemon sends before reading it', async () => {
  render(App);
  await listed();
  const events = vi.mocked(listen).mock.calls.map((c) => c[0]);
  for (const name of [
    'daemon:status',
    'daemon:state',
    'daemon:downloads',
    'daemon:down',
    'app:reopened',
  ]) {
    expect(events).toContain(name);
  }
});

it('starts the daemon itself when it finds it gone', async () => {
  vi.mocked(status).mockRejectedValue({ message: 'not running' });
  render(App);
  await waitFor(() => expect(vi.mocked(startDaemon)).toHaveBeenCalled());
  await waitFor(() => expect(screen.getByText('Banshee is not running')).toBeTruthy());
});

it('says why the daemon would not start', async () => {
  const refusal =
    'Banshee did not start: Delete `preset` under [stt]. Fix that, then run banshee start again.';
  vi.mocked(status).mockRejectedValue({ message: 'not running' });
  vi.mocked(startDaemon).mockRejectedValue({ message: refusal });
  render(App);
  await waitFor(() => expect(screen.getByText(refusal)).toBeTruthy());
  expect(screen.getByText('Banshee is not running')).toBeTruthy();
});

it('reads the voices and the agents the strip needs', async () => {
  render(App);
  await waitFor(() => expect(vi.mocked(listVoices)).toHaveBeenCalled());
  await waitFor(() => expect(vi.mocked(detectAgents)).toHaveBeenCalled());
  // Neither read may take the status and the history down with it.
  expect(await listed()).toBeTruthy();
});

it('keeps the rows listed when the daemon says it is no longer saving', async () => {
  render(App);
  await listed();

  // The daemon reports both once it takes the switch.
  const off = {
    ...ready,
    history_enabled: false,
    config: { ...ready.config, daemon: { save_history: false } },
  };
  daemon.update((s) => reduceStatus(s, off));
  const notices = await screen.findByRole('region', { name: 'Notices' });
  expect(within(notices).getByText('Save history is off, so nothing new is kept.')).toBeTruthy();
  expect(await listed()).toBeTruthy();
});

it('counts the rows Saved history keeps', async () => {
  render(App);
  await listed();
  await choose('Saved history');
  await waitFor(() =>
    expect(pageHeading('Saved history').textContent).toBe(
      'Banshee is keeping 2 rows, on this machine.',
    ),
  );
});

it('says what stays when saving is off and rows exist', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, history_enabled: false });
  render(App);
  await listed();
  await fireEvent.click(screen.getByRole('button', { name: 'Open Saved history' }));
  await waitFor(() =>
    expect(screen.getByText(/What it kept stays until you clear it/)).toBeTruthy(),
  );
  expect(pageHeading('Saved history').textContent).toBe('Banshee is keeping nothing new you say.');
});

it('a question with no answer reaches the record without a transcription', async () => {
  render(App);
  await listed();

  vi.mocked(history).mockResolvedValue([
    ...rows,
    asked(3, { question: 'Which branch should I use?', outcome: 'no_answer' }),
  ]);
  pushes.get('daemon:state')?.({ payload: { history_changes: 1 } });

  expect(
    await screen.findByRole('option', {
      name: /^Agent asked Which branch should I use\?, .*, No answer heard$/,
    }),
  ).toBeTruthy();
});

it('search finds a question by its question', async () => {
  vi.mocked(history).mockResolvedValue([
    ...rows,
    asked(3, { question: 'Which branch should I use?', outcome: 'no_answer' }),
  ]);
  render(App);
  await screen.findByRole('option', { name: /Which branch should I use/ });
  await fireEvent.keyDown(window, { key: 'f', metaKey: true });
  await fireEvent.input(await screen.findByRole('searchbox'), { target: { value: 'branch' } });
  await waitFor(() => expect(screen.getAllByRole('option').length).toBe(1));
  expect(screen.getByRole('option', { name: /Which branch should I use/ })).toBeTruthy();
});

it('says no stream is open, rather than that there is no microphone', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    audio_device: null,
    download_megabytes: 860,
    blockers: [
      {
        kind: 'model',
        id: 'ggml-large-v3-turbo-q5_0.bin',
        name: 'ggml-large-v3-turbo-q5_0.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'recording, dictation, and ask_user do not work',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);

  // The page says whether any stream is open. Nothing says the machine has no
  // microphone.
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
  expect(screen.queryByText(/no microphone/i)).toBeNull();

  await choose('Listening');
  await waitFor(() => expect(page('Listening')).toBeTruthy());
  expect(pageHeading('Listening').textContent).toContain('Banshee is not listening yet');
});

it('asks which listening model to fetch before it fetches one', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    download_megabytes: 860,
    blockers: [
      {
        kind: 'model',
        id: 'ggml-large-v3-turbo-q5_0.bin',
        name: 'ggml-large-v3-turbo-q5_0.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'recording, dictation, and ask_user do not work',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);

  // The model decides what the run costs, and nothing else asks. The daemon
  // answers with what is still missing rather than a fixed total.
  await waitFor(() =>
    expect(screen.getByRole('radiogroup', { name: 'Listening model' })).toBeTruthy(),
  );
  expect(screen.getByRole('radio', { name: 'Parakeet Unified' }).getAttribute('aria-checked')).toBe(
    'true',
  );
  expect(screen.getByText(/About 860 MB to fetch/)).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy();

  // The daemon blocks on two files and the run fetches four, so a count here
  // contradicts the `1 of 4` the progress line shows a moment later.
  expect(screen.getByText('Banshee needs its models')).toBeTruthy();
  expect(screen.queryByText(/files are missing|file is missing/)).toBeNull();
});

it('offers a restart, not a download, when the files are already there', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'recording_pipeline',
        name: 'Banshee needs a restart',
        remedy: 'restart',
        consequence: 'a model would not load',
        fix: 'restart it: banshee start',
        command: 'banshee start',
      },
    ],
  });
  render(App);

  // The daemon calls this a model fault, but its fix is `banshee start`. Routing
  // on the kind offered Download for files that are on disk.
  await waitFor(() => expect(screen.getByRole('button', { name: 'Restart Banshee' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();

  // Every file is on disk in this state, so counting it as a missing file lies.
  expect(screen.getByText('Banshee needs a restart')).toBeTruthy();
  expect(screen.queryByText(/file is missing|files are missing/)).toBeNull();
  expect(screen.queryByRole('radiogroup', { name: 'Listening model' })).toBeNull();

  // `startDaemon` is for a daemon that is down. This blocker is offered while
  // the daemon runs, so only a restart clears it.
  vi.mocked(status).mockResolvedValue({ ...ready, blockers: [] });
  await fireEvent.click(screen.getByRole('button', { name: 'Restart Banshee' }));
  expect(vi.mocked(restartDaemon)).toHaveBeenCalled();
  expect(vi.mocked(startDaemon)).not.toHaveBeenCalled();

  // Nothing pushes a blocker change, so a restart that does not re-read leaves
  // the box asking for the restart it has just been given.
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Restart Banshee' })).toBeNull());
});

it('says which file is downloading and how far it has come', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'ggml-large-v3-turbo-q5_0.bin',
        name: 'ggml-large-v3-turbo-q5_0.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'recording, dictation, and ask_user do not work',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());

  pushes.get('daemon:downloads')?.({
    payload: {
      model: 'ggml.bin',
      label: 'Listening model',
      index: 1,
      count: 4,
      bytes: 40,
      total: 100,
      state: 'downloading',
    },
  });
  await waitFor(() => expect(screen.getByText('Listening model, 1 of 4 · 40%')).toBeTruthy());

  // The two blocking files land before the other two, so the daemon starts
  // asking for a restart while the run is still going. One thing is happening,
  // and the box says which.
  expect(screen.getByText(/Getting Banshee's models/)).toBeTruthy();
  expect(screen.queryByRole('radiogroup', { name: 'Listening model' })).toBeNull();

  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'recording_pipeline',
        name: 'Banshee needs a restart',
        remedy: 'restart',
        consequence: 'a model would not load',
        fix: 'restart it: banshee start',
        command: 'banshee start',
      },
    ],
  });
  daemon.update((state) =>
    reduceStatus(state, {
      ...ready,
      running: true,
      blockers: [
        {
          kind: 'model',
          id: 'recording_pipeline',
          name: 'Banshee needs a restart',
          remedy: 'restart',
          consequence: 'a model would not load',
          fix: 'restart it: banshee start',
          command: 'banshee start',
        },
      ],
    }),
  );
  await waitFor(() => expect(screen.getByText('Listening model, 1 of 4 · 40%')).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Restart Banshee' })).toBeNull();
});

it('re-reads the daemon when a download ends, so the box stops asking', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'ggml-large-v3-turbo-q5_0.bin',
        name: 'ggml-large-v3-turbo-q5_0.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'recording, dictation, and ask_user do not work',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());

  // Files landing changes what the daemon is blocked on, and no other push
  // says so.
  vi.mocked(status).mockResolvedValue({ ...ready, blockers: [] });
  pushes.get('daemon:downloads')?.({
    payload: { model: 'ggml.bin', bytes: 100, total: 100, state: 'done' },
  });
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Download' })).toBeNull());
});

it('offers nothing to stop before anything has been said', async () => {
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() => expect(screen.getByText('Nothing here yet')).toBeTruthy());

  // The switch would offer to stop a thing that has never started.
  expect(screen.queryByRole('button', { name: 'Stop saving' })).toBeNull();
  expect(screen.queryByText(/Nothing saved yet/)).toBeNull();
});

it('still says so when it is keeping nothing new and holds nothing', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, history_enabled: false });
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  expect(
    await screen.findByRole('heading', { name: 'Save history is off, so nothing new is kept.' }),
  ).toBeTruthy();
});

it('offers no command to copy, because every fix is a button', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'ggml.bin',
        name: 'ggml.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'x',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'banshee setup' })).toBeNull();
});

// The voice files arrive with the download, and the last one a run fetches is a
// voice, so the list read at startup is answered before any of them exist.
it('reads the voices again once their files have landed', async () => {
  vi.mocked(listVoices).mockResolvedValue({ voices: [], current: null });
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'ggml.bin',
        name: 'ggml.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'x',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
  await choose('Speaking');
  await waitFor(() => expect(pageHeading('Speaking').textContent).toContain('af_sky'));

  vi.mocked(status).mockResolvedValue({ ...ready, blockers: [] });
  vi.mocked(listVoices).mockResolvedValue({
    voices: [{ id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true }],
    current: 'af_sky',
  });
  pushes.get('daemon:downloads')?.({
    payload: {
      model: 'af_sky.bin',
      label: 'Voice',
      index: 4,
      count: 4,
      bytes: 1,
      total: 1,
      state: 'done',
    },
  });

  // The page names the voice, rather than the file the daemon fetched.
  await waitFor(() => expect(pageHeading('Speaking').textContent).toBe('Banshee speaks as Sky.'));
});

// A window that can only offer what is on disk cannot offer a choice at all.
it('offers every voice, and fetches the one that is chosen', async () => {
  vi.mocked(listVoices).mockResolvedValue({
    current: 'af_sky',
    voices: [
      { id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true },
      { id: 'bm_george', name: 'George', description: 'British, steady', downloaded: false },
    ],
  });
  render(App);
  await listed();
  await choose('Speaking');

  await waitFor(() => expect(pageHeading('Speaking')).toBeTruthy());
  expect(screen.getByText('George')).toBeTruthy();

  expect(screen.queryByRole('button', { name: 'Play George' })).toBeNull();
  expect(screen.getByRole('button', { name: /Get George/ })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Play Sky' }).hasAttribute('disabled')).toBe(false);

  await fireEvent.click(screen.getByRole('button', { name: /Get George/ }));
  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.voice', 'bm_george'));
  expect(vi.mocked(downloadModels)).toHaveBeenCalled();
});

it('does not fetch a voice that is already here', async () => {
  vi.mocked(listVoices).mockResolvedValue({
    current: 'bm_george',
    voices: [
      { id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true },
      { id: 'bm_george', name: 'George', description: 'British, steady', downloaded: true },
    ],
  });
  render(App);
  await listed();
  await choose('Speaking');
  await waitFor(() => expect(pageHeading('Speaking')).toBeTruthy());

  await fireEvent.change(screen.getByRole('radio', { name: /Sky/ }));
  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.voice', 'af_sky'));
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
});

// Choosing a voice that is not here writes it, and the daemon refuses until the
// file lands, so `tts.voice` waits for the second the fetch takes. The download
// speaks for it while it runs, and the daemon applies it when the run ends.
it('leaves a voice that waits for its file to the download while one runs', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['tts.voice'] });
  render(App);
  await waitFor(() => expect(item('Speaking').textContent).toContain(WAITS_FOR_ITS_FILE));

  pushes.get('daemon:downloads')?.({
    payload: {
      model: 'bm_george.bin',
      label: 'Voice',
      index: 1,
      count: 1,
      bytes: 1,
      total: 2,
      state: 'downloading',
    },
  });
  await waitFor(() => expect(item('Speaking').textContent).not.toContain(WAITS_FOR_ITS_FILE));
});

// The row on the page says the same sentence, so the two cannot disagree.
it('says a plain waiting key could not be used yet', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['tts.remote.model'] });
  render(App);
  await waitFor(() => expect(item('Speaking').textContent).toContain(WAITING_SAYS));
  expect(item('Speaking').textContent).not.toContain(WAITS_FOR_ITS_FILE);
});

// A run reports once per file. A download writes no history, so the record
// cannot have moved and only the status is worth asking for again.
it('does not re-read the record for every file a download finishes', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'ggml.bin',
        name: 'ggml.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'x',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());

  const readsAtStart = vi.mocked(history).mock.calls.length;
  for (let file = 1; file <= 3; file++) {
    pushes.get('daemon:downloads')?.({
      payload: { model: `f${file}.bin`, index: file, count: 4, bytes: 1, total: 1, state: 'done' },
    });
  }
  await waitFor(() => expect(vi.mocked(status).mock.calls.length).toBeGreaterThan(1));
  expect(vi.mocked(history).mock.calls.length).toBe(readsAtStart);
});

// A grant is made in System Settings whatever else is arriving. Hiding it
// behind a download leaves no route to the pane for the length of the fetch,
// and a first run is exactly when both are outstanding.
it('keeps a permission reachable while a download runs', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'permission',
        id: 'accessibility',
        name: 'Accessibility is not granted',
        consequence: 'dictation cannot type and the hotkey stays inert',
        fix: 'grant it in System Settings',
      },
      {
        kind: 'model',
        id: 'ggml.bin',
        name: 'ggml.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'x',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /^Open System Settings for / })).toBeTruthy(),
  );

  pushes.get('daemon:downloads')?.({
    payload: {
      model: 'ggml.bin',
      label: 'Listening model',
      index: 1,
      count: 4,
      bytes: 1,
      total: 2,
      state: 'downloading',
    },
  });
  await waitFor(() => expect(screen.getByText(/Getting Banshee's models/)).toBeTruthy());
  expect(screen.getByRole('button', { name: /^Open System Settings for / })).toBeTruthy();
});

// A refused call answers -32005 rather than throwing on the wire, and an
// unhandled rejection leaves the button looking inert with nothing said.
it('says so when an action is refused', async () => {
  vi.mocked(downloadModels).mockRejectedValue(new Error('A download is already running.'));
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'ggml.bin',
        name: 'ggml.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'x',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());

  await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
  await waitFor(() => expect(screen.getByText(/Download did not work/)).toBeTruthy());
});

// No arriving file answers `audio.hotkey`, so a download must not quiet the
// mark that says the daemon could not use it.
it('still marks a key no download can settle', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    pending: ['audio.hotkey'],
    blockers: [
      {
        kind: 'model',
        id: 'ggml.bin',
        name: 'ggml.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'x',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(item('Hotkey').textContent).toContain(WAITING_SAYS));
});

it('reaches Saved history before anything has been said', async () => {
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() => expect(screen.getByText('Nothing here yet')).toBeTruthy());

  await choose('Saved history');
  await waitFor(() => expect(pageHeading('Saved history')).toBeTruthy());
  expect(screen.getByRole('button', { name: /Stop saving/ })).toBeTruthy();
});

// A microphone fault and a model fault carry the same command, and restarting
// does nothing for a device that is not plugged in.
it('does not call an unplugged microphone a restart', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'pipeline',
        id: 'recording_pipeline',
        name: 'The microphone is not working',
        remedy: 'restart',
        consequence: 'the microphone would not open: no device',
        fix: 'connect the microphone, grant it in Privacy & Security, or fix [audio] input_device. If recording does not recover on its own, restart: banshee start',
        command: 'banshee start',
      },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByText('The microphone is not working')).toBeTruthy());

  expect(screen.queryByText('Banshee needs a restart')).toBeNull();
  expect(screen.getByRole('button', { name: 'Restart anyway' })).toBeTruthy();
  expect(screen.getByText(/onnect the microphone/)).toBeTruthy();
});

// A page replaces History. The notice's button is destroyed by the click it is
// answering, so the keyboard is left on a node that is gone.
it('puts the keyboard on the page a notice opens', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, history_enabled: false });
  render(App);
  await listed();

  const opener = screen.getByRole('button', { name: 'Open Saved history' });
  opener.focus();
  await fireEvent.click(opener);

  await waitFor(() => expect(page('Saved history')).toBeTruthy());
  expect(document.activeElement).toBe(pageHeading('Saved history'));
});

// A page has no Escape action: the sidebar is the way out.
it('keeps the page and the focus on Escape', async () => {
  render(App);
  await listed();

  const opener = item('Speaking');
  opener.focus();
  await fireEvent.click(opener);
  await waitFor(() => expect(page('Speaking')).toBeTruthy());
  expect(document.activeElement).toBe(pageHeading('Speaking'));

  await fireEvent.keyDown(window, { key: 'Escape' });
  expect(page('Speaking')).toBeTruthy();
  expect(document.activeElement).toBe(pageHeading('Speaking'));
});

// The key sat on the same ink underline the pickers use, so it read as a field
// to type into while being a span.
it('lets the hotkey be changed by the key it names', async () => {
  render(App);
  await listed();
  await choose('Hotkey');

  const key = await screen.findByRole('button', { name: 'Right Command, change the hotkey' });
  await fireEvent.click(key);
  await waitFor(() => expect(screen.getByText('Press a key')).toBeTruthy());
});

// The differentiating moment cannot be the window's quietest pixel.
it('says an agent is waiting above the list, and names the key that answers it', async () => {
  render(App);
  await listed();
  // Armed arrives as a live push mid-session, after the config is known.
  daemon.update((s) => ({ ...s, live: { ...s.live, armed: true } }));
  // hotkey_mode is toggle in `ready.json`, so the verb is Tap, not Hold.
  const notices = await screen.findByRole('region', { name: 'Notices' });
  expect(
    within(notices).getByText('An agent asked a question and is waiting for your answer.'),
  ).toBeTruthy();
  // Only the key name is a machine value, so only it takes the mono face.
  const how = within(notices).getByText(
    (_, node) => node?.tagName === 'P' && node.textContent === 'Tap Right Command to answer.',
  );
  expect(within(how).getByText('Right Command').classList.contains('mono')).toBe(true);
});

// Each item is its own Tab stop, so no item hides behind another.
it('makes every sidebar item a Tab stop that opens nothing on focus', async () => {
  render(App);
  await listed();

  const items = within(sidebar()).getAllByRole('button');
  expect(items.map((one) => one.tabIndex)).toEqual(items.map(() => 0));

  items[3].focus();
  await fireEvent.keyDown(items[3], { key: 'ArrowDown' });
  expect(page('History')).toBeTruthy();
});

// Nobody needs 21 steps of a value the window only ever reads back as one of
// three words. The control shows the band the current float falls in.
it('sets sensitivity by band, and writes a float for it', async () => {
  render(App);
  await listed();
  await choose('Listening');

  const group = await screen.findByRole('radiogroup', { name: 'Sensitivity' });
  expect(group).toBeTruthy();
  // ready.json holds 0.5, which is the middle band.
  expect(screen.getByRole('radio', { name: 'Medium' }).getAttribute('aria-checked')).toBe('true');

  await fireEvent.click(screen.getByRole('radio', { name: 'High' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('stt.vad_threshold', 0.85));
});

// tts.fallback serves no job this audience has, and belongs to the CLI.
it('does not spend the Speaking page on a key that belongs to the CLI', async () => {
  render(App);
  await listed();
  await choose('Speaking');

  await waitFor(() => expect(screen.getByRole('radio', { name: /Sky/ })).toBeTruthy());
  expect(screen.queryByText(/If a voice is missing/i)).toBeNull();
  expect(screen.queryByRole('radiogroup', { name: /voice is missing/i })).toBeNull();
});

// Software that is not on this machine has no action, so it earns no row.
it('collapses agents that are not installed into one line', async () => {
  vi.mocked(detectAgents).mockResolvedValue([
    { id: 'claude', name: 'Claude Code', presence: 'connected', note: '' },
    { id: 'cursor', name: 'Cursor', presence: 'found', note: '' },
    { id: 'pi', name: 'Pi', presence: 'absent', note: '' },
    { id: 'antigravity', name: 'Antigravity', presence: 'absent', note: '' },
  ]);
  render(App);
  await listed();
  await choose('Agents');

  await waitFor(() => expect(screen.getByText('Claude Code')).toBeTruthy());
  expect(screen.getByText(/also works with/)).toBeTruthy();
  expect(screen.queryByText(/^Not installed$/)).toBeNull();
});

// Keeping is the primary, and the prompt names how much would go.
it('makes keeping the record the primary, and names what would be lost', async () => {
  render(App);
  await listed();
  await choose('Saved history');
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));

  expect(screen.getByText(/Delete all 2 rows\?/)).toBeTruthy();
  const buttons = screen.getAllByRole('button').map((b) => b.textContent?.trim());
  expect(buttons.indexOf('Keep it')).toBeLessThan(buttons.indexOf('Delete everything'));
});

// Abandoning a half-typed word must not also throw the reader off the page.
// The field claims the keyboard for exactly this, and releasing the claim
// inside the same keydown lets the event reach the window handler unclaimed.
it('keeps the page open when Escape abandons a vocabulary word', async () => {
  render(App);
  await listed();
  await choose('Listening');
  await fireEvent.click(await screen.findByRole('button', { name: 'Add a word' }));

  const field = screen.getByRole('textbox', { name: 'New word' });
  await fireEvent.keyDown(field, { key: 'Escape' });

  expect(screen.queryByRole('region', { name: 'Listening' })).toBeTruthy();
});

// A hand-edited config, or one written by a newer daemon, holds values these
// options do not. Every cell at -1 puts the group out of the keyboard's reach.
it('keeps a tab stop when no option matches the daemon', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: { ...ready.config, audio: { ...ready.config.audio, barge_in: 'something-else' } },
  });
  render(App);
  await listed();
  await choose('Hotkey');

  const group = await screen.findByRole('radiogroup', { name: 'While Banshee is talking' });
  const stops = [...group.querySelectorAll('button')].map((b) => (b as HTMLElement).tabIndex);
  expect(stops).toContain(0);
});

// A live region alone leaves a sighted reader nothing, hence the .sr assertion.
it('draws a failure where a reader can see it, and keeps it there', async () => {
  vi.mocked(copyText).mockRejectedValue(new Error('no clipboard'));
  render(App);
  await listed();

  await fireEvent.click(screen.getAllByRole('button', { name: /^Copy/ })[0]);
  const shown = await screen.findByText(/Nothing was copied/);
  expect(shown.closest('.sr')).toBeNull();

  // Well past the 1500ms after which the sibling confirmation clears itself.
  vi.advanceTimersByTime(5000);
  await waitFor(() => expect(screen.getByText(/Nothing was copied/)).toBeTruthy());
});

it('lets a failure be dismissed', async () => {
  vi.mocked(clearHistory).mockRejectedValue(new Error('locked'));
  render(App);
  await listed();
  await choose('Saved history');
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete everything' }));

  await waitFor(() => expect(screen.getByText(/could not be cleared/)).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
  await waitFor(() => expect(screen.queryByText(/could not be cleared/)).toBeNull());
});

// A live region speaks a change, so the same failure again has to leave the
// region and come back for a reader to hear it.
it('speaks the same failure a second time', async () => {
  render(App);
  await listed();
  report('The audio did not play.');
  const first = await screen.findByText('The audio did not play.');
  report('The audio did not play.');
  await waitFor(() => expect(screen.getByText('The audio did not play.')).not.toBe(first));
});

it('speaks the same failure reported three times in a row', async () => {
  render(App);
  await listed();
  report('The audio did not play.');
  const first = await screen.findByText('The audio did not play.');
  report('The audio did not play.');
  report('The audio did not play.');
  await waitFor(() => expect(screen.getByText('The audio did not play.')).not.toBe(first));
});

it('opens search on Cmd+F whatever Caps Lock says', async () => {
  render(App);
  await listed();
  await fireEvent.keyDown(window, { key: 'F', metaKey: true });
  const find = await screen.findByRole('searchbox', { name: 'Search' });
  await waitFor(() => expect(document.activeElement).toBe(find));
});

// Activating Clear destroys the button that was activated. Without a move the
// focus falls to the body, so a keyboard reader has to Tab from the top of the
// document to reach the question they just asked for. axe cannot see this.
it('moves focus into the confirm, and back out of it', async () => {
  render(App);
  await listed();
  await choose('Saved history');

  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));
  const keep = await screen.findByRole('button', { name: 'Keep it' });
  expect(document.activeElement).toBe(keep);

  await fireEvent.click(keep);
  expect(document.activeElement).toBe(await screen.findByRole('button', { name: 'Clear' }));
});

// A successful delete removes the Clear button it came from, so sending focus
// back there drops it as surely as never moving it.
it('keeps focus somewhere real after the record is deleted', async () => {
  render(App);
  await listed();
  await choose('Saved history');
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));
  // The daemon keeps nothing once it has cleared, so a re-read finds nothing.
  vi.mocked(history).mockResolvedValue([]);
  await fireEvent.click(await screen.findByRole('button', { name: 'Delete everything' }));

  await waitFor(() => expect(screen.queryByRole('button', { name: 'Clear' })).toBeNull());
  expect(document.activeElement).not.toBe(document.body);
});

// The round trip is not instant. A second press on a live Delete ran the whole
// thing again, took the client lock twice, and stole focus back afterwards.
it('takes one delete however many times the button is pressed', async () => {
  let release: () => void = () => {};
  vi.mocked(clearHistory).mockReturnValue(new Promise<void>((resolve) => (release = resolve)));
  render(App);
  await listed();
  await choose('Saved history');
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));

  const destroy = await screen.findByRole('button', { name: 'Delete everything' });
  await fireEvent.click(destroy);
  await fireEvent.click(destroy);
  release();
  await waitFor(() => expect(vi.mocked(clearHistory).mock.calls.length).toBe(1));
});

// The heading reads the daemon for what is in force and the config for what was
// asked, so the four forms cannot contradict the page below them.
it('says where text goes in the Speaking heading', async () => {
  const remoteSpeaker = {
    ...ready,
    config: {
      ...ready.config,
      tts: {
        ...ready.config.tts,
        provider: 'remote',
        remote: {
          base_url: 'https://api.openai.com/v1',
          model: 'gpt-4o-mini-tts',
          voice: 'marin',
          instructions: '',
        },
      },
    },
    remote: {
      stt: { remote: false, host: null, key_present: false },
      tts: { remote: true, host: 'api.openai.com', speaker_started: true, key_present: true },
    },
  };
  vi.mocked(status).mockResolvedValue(remoteSpeaker);
  render(App);

  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() =>
    expect(screen.getByText('Banshee speaks through api.openai.com as marin.')).toBeTruthy(),
  );
});

// Kokoro speaks for every user who has not set a remote speaker up, so the two
// local arms of the heading are the ones most people read.
it('names the local voice Kokoro speaks in', async () => {
  render(App);
  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() => expect(screen.getByText('Banshee speaks as Sky.')).toBeTruthy());
});

it('says a local speaker with no voice set has none yet', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: { ...ready.config, tts: { ...ready.config.tts, voice: '' } },
  });
  render(App);
  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() => expect(screen.getByText('Banshee has no voice yet.')).toBeTruthy());
});

// A remote speaker with no voice named refuses to start, so nothing is spoken
// and nothing leaves the machine. The heading names the one thing that fixes it.
it('says a remote speaker with no voice cannot speak', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: {
      ...ready.config,
      tts: {
        ...ready.config.tts,
        provider: 'remote',
        remote: {
          base_url: 'https://api.openai.com/v1',
          model: 'tts-1',
          voice: '',
          instructions: '',
        },
      },
    },
    remote: {
      stt: { remote: false, host: null, key_present: false },
      tts: { remote: true, host: 'api.openai.com', speaker_started: false, key_present: true },
    },
  });
  render(App);
  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() =>
    expect(
      screen.getByText('Banshee cannot speak through api.openai.com until you name a voice.'),
    ).toBeTruthy(),
  );
});

// A remote speaker with no key speaks nothing, so the heading may not say it
// speaks. It names the one thing that fixes it.
it('says a remote speaker with no key cannot speak', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: {
      ...ready.config,
      tts: {
        ...ready.config.tts,
        provider: 'remote',
        remote: {
          base_url: 'https://api.openai.com/v1',
          model: 'tts-1',
          voice: 'marin',
          instructions: '',
        },
      },
    },
    remote: {
      stt: { remote: false, host: null, key_present: false },
      tts: { remote: true, host: 'api.openai.com', speaker_started: false, key_present: false },
    },
  });
  render(App);
  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() =>
    expect(
      screen.getByText('Banshee cannot speak through api.openai.com until you paste a key.'),
    ).toBeTruthy(),
  );
});

// A key pasted and a voice named still leave the speaker unstarted when the
// server refuses the daemon at startup. The heading may not say it speaks, and
// there is no one field to name as the fix.
it('says a remote speaker that never started cannot speak', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: {
      ...ready.config,
      tts: {
        ...ready.config.tts,
        provider: 'remote',
        remote: {
          base_url: 'https://api.openai.com/v1',
          model: 'tts-1',
          voice: 'marin',
          instructions: '',
        },
      },
    },
    remote: {
      stt: { remote: false, host: null, key_present: false },
      tts: { remote: true, host: 'api.openai.com', speaker_started: false, key_present: true },
    },
  });
  render(App);
  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() =>
    expect(screen.getByText('Banshee cannot speak through api.openai.com.')).toBeTruthy(),
  );
});

// The daemon chose its speaker at startup, so the live table can lose the name
// while that speaker still takes every reply. The reply says one is speaking
// and the table has no name for it, so the sentence names the host alone.
it('names the host alone when a started speaker has lost its name', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: {
      ...ready.config,
      tts: {
        ...ready.config.tts,
        provider: 'remote',
        remote: {
          base_url: 'https://api.openai.com/v1',
          model: 'tts-1',
          voice: '',
          instructions: '',
        },
      },
    },
    remote: {
      stt: { remote: false, host: null, key_present: false },
      tts: { remote: true, host: 'api.openai.com', speaker_started: true, key_present: true },
    },
  });
  render(App);
  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() =>
    expect(screen.getByText('Banshee speaks through api.openai.com.')).toBeTruthy(),
  );
});

// The first line a person reads names every machine their words touch. A
// speaker that did not start sends the text nowhere, so naming its host there
// would claim an egress this machine does not have.
it('names no speaker host on a first run when the speaker did not start', async () => {
  vi.mocked(history).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue({
    ...permissions,
    blockers: permissions.blockers as Blocker[],
    remote: {
      stt: { remote: true, host: 'api.groq.com', key_present: true },
      tts: { remote: true, host: 'api.openai.com', speaker_started: false, key_present: false },
    },
  });
  render(App);
  const opening = await waitFor(() => screen.getByText(/types what you say/));
  expect(opening.textContent).toContain('what you say goes to api.groq.com to be heard');
  expect(opening.textContent).not.toContain('api.openai.com');
});

// The Microphone lead already says Banshee cannot reach the remote listener
// once a recording_pipeline blocker stops it, so the opening must not repeat
// a promise the daemon just broke.
it('names no listening host on a first run when the pipeline blocker stops the listener', async () => {
  vi.mocked(history).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue({
    ...remote,
    blockers: [
      {
        kind: 'provider',
        id: 'recording_pipeline',
        name: 'Remote transcription',
        remedy: 'restart',
        consequence: 'dictation cannot reach the remote server',
        fix: 'check the remote server',
      },
    ] as Blocker[],
  });
  render(App);
  const opening = await waitFor(() => screen.getByText(/types what you say/));
  expect(opening.textContent).not.toContain('goes to');
});

// The voice is not missing when the daemon is down, so the heading says which
// of the two is wrong.
it('blames the stopped daemon rather than the voice', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, running: false });
  render(App);
  await fireEvent.click(await waitFor(() => item('Speaking')));
  await waitFor(() =>
    expect(screen.getByText('Banshee is not running, so nothing is spoken.')).toBeTruthy(),
  );
});

// The cell reads every key under the speaker's table, so a key added later
// cannot be left off a hand-written list and lose its mark. A key of another
// job's is the discriminating half: it marks that job's cell instead.
const VOICE_WAITS: [string, boolean][] = [
  ['tts.voice', true],
  ['tts.remote.base_url', true],
  ['tts.remote.model', true],
  ['tts.remote.voice', true],
  ['tts.remote.instructions', true],
  ['tts.remote.api_key', true],
  ['tts.remote.response_format', true],
  ['tts.remote.sample_rate', true],
  ['stt.model', false],
  ['audio.hotkey', false],
];

it.each(VOICE_WAITS)('marks the Speaking item for %s: %s', async (key, marked) => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: [key] });
  render(App);
  // The row arrives after the status it follows, so an unmarked item is not
  // merely one nothing has reached yet.
  await listed();
  if (marked) {
    await waitFor(() => expect(item('Speaking').textContent).toContain(waitingSays(key)));
  } else expect(item('Speaking').textContent).not.toContain('Set. Banshee');
});

// A stopped daemon has no key to apply, so a mark would point at nothing.
it('marks no waiting voice key while the daemon is down', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    running: false,
    pending: ['tts.remote.model'],
  });
  render(App);
  await waitFor(() => expect(screen.getByText('Banshee is not running')).toBeTruthy());
  expect(item('Speaking').textContent).not.toContain(WAITING_SAYS);
});

it('marks the voice in force when the speaker asked for could not start', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: {
      ...ready.config,
      tts: {
        ...ready.config.tts,
        provider: 'remote',
        remote: {
          base_url: 'https://api.openai.com/v1',
          model: 'tts-1',
          voice: 'marin',
          instructions: '',
        },
      },
    },
    remote: {
      stt: { remote: false, host: null, key_present: false },
      tts: { remote: false, host: null, speaker_started: true, key_present: true },
    },
    pending: ['tts.provider'],
  });
  render(App);
  await waitFor(() => expect(item('Speaking').textContent).toContain(WAITING_SAYS));
});

// Wayland grants no global grab, so the daemon binds nothing and says so.
// Every place the window named a key was a promise nobody could keep.
const onWayland = { ...ready, hotkey_listens: false };

it('names the compositor on the Hotkey page, not a key nobody listens for', async () => {
  vi.mocked(status).mockResolvedValue(onWayland);
  render(App);
  await listed();

  await choose('Hotkey');
  const heading = await waitFor(() => pageHeading('Hotkey'));
  expect(heading.textContent).toMatch(/compositor/);
  expect(heading.textContent).not.toMatch(/Right Command/);
});

it('offers the compositor commands instead of a capture that writes nothing', async () => {
  vi.mocked(status).mockResolvedValue(onWayland);
  render(App);
  await listed();
  await choose('Hotkey');

  expect(await screen.findByText(/^banshee record start banshee record stop$/)).toBeTruthy();
  // The capture writes `audio.hotkey`, and no listener reads it in this session.
  expect(screen.queryByRole('button', { name: /change the hotkey/ })).toBeNull();
});

it('sends the answerer to the compositor binding while an agent waits', async () => {
  vi.mocked(status).mockResolvedValue(onWayland);
  render(App);
  await listed();
  daemon.update((s) => ({ ...s, live: { ...s.live, armed: true } }));

  await waitFor(() =>
    expect(screen.getByText(/compositor's Banshee binding to answer/)).toBeTruthy(),
  );
  expect(screen.queryByText(/Tap Right Command to answer/)).toBeNull();
});

// A failure reported by the daemon lives on a page nobody has open, so the
// item that opens that page carries the mark.
it('marks Speaking for a failed reply, and opens it from the note above History', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();
  daemon.update((s) => ({
    ...s,
    live: { ...s.live, last_speech_error: 'api.deepinfra.com says no' },
  }));

  await waitFor(() =>
    expect(
      within(sidebar()).getByRole('button', { name: 'Speaking needs attention' }),
    ).toBeTruthy(),
  );
  expect(within(sidebar()).getByRole('button', { name: 'Listening' })).toBeTruthy();

  await fireEvent.click(screen.getByRole('button', { name: 'Open Speaking' }));
  await waitFor(() => expect(pageHeading('Speaking')).toBeTruthy());
  expect(document.getElementById('speech-failure')?.textContent).toContain(
    'api.deepinfra.com says no',
  );
});

it('marks Listening for a failed dictation, and its item opens that page', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();
  daemon.update((s) => ({ ...s, live: { ...s.live, last_error: 'api.groq.com says no' } }));

  await fireEvent.click(await waitFor(() => item('Listening needs attention')));
  await waitFor(() => expect(document.activeElement).toBe(pageHeading('Listening')));
});

// The mark names the page in two words, so History states the reason and the
// fix where the reader already looks.
it('states a failed dictation and its fix inline, above the record', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  const { container } = render(App);
  await listed();
  daemon.update((s) => ({ ...s, live: { ...s.live, last_error: 'api.groq.com says no' } }));

  await waitFor(() => {
    const line = container.querySelector('.failure')?.textContent ?? '';
    expect(line).toContain('api.groq.com says no');
  });
  expect(screen.getByRole('button', { name: 'Open Listening' })).toBeTruthy();
  expect(container.querySelector('.failure')?.textContent).not.toContain('Microphone');
  expect(within(sidebar()).getByRole('button', { name: 'Listening needs attention' })).toBeTruthy();
});

// The note has one place, and dictation is the way in. Each mark is its own.
it('carries the dictation failure in the note, and marks both when both fail', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();
  daemon.update((s) => ({
    ...s,
    live: { ...s.live, last_error: 'no microphone', last_speech_error: 'no voice' },
  }));

  expect(await screen.findByRole('button', { name: 'Open Listening' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Open Speaking' })).toBeNull();
  expect(within(sidebar()).getByRole('button', { name: 'Listening needs attention' })).toBeTruthy();
  expect(within(sidebar()).getByRole('button', { name: 'Speaking needs attention' })).toBeTruthy();
});

it('marks no item when the daemon reports no failure', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();
  expect(within(sidebar()).queryByRole('button', { name: /needs attention/ })).toBeNull();
});

it('speaks a failure that arrives while its page is not open', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();
  expect(get(announcement)).toBe('');

  daemon.update((s) => ({
    ...s,
    live: { ...s.live, last_speech_error: 'api.openai.com did not answer in time' },
  }));
  await waitFor(() =>
    expect(get(announcement)).toBe(
      'The last spoken reply failed. api.openai.com did not answer in time',
    ),
  );
});

// On a remote speaker the failure sits below the fold, so opening the page is
// only half of what the note promises.
it('lands on the failure itself, not the top of the page it opens', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();
  daemon.update((s) => ({ ...s, live: { ...s.live, last_speech_error: 'no voice' } }));

  await fireEvent.click(await screen.findByRole('button', { name: 'Open Speaking' }));
  await waitFor(() =>
    expect(document.activeElement).toBe(document.getElementById('speech-failure')),
  );
});

// Seven sidebar stops come first in the tab order, so the keyboard needs a way
// past them to the page itself.
it('the skip link enters the page that is showing', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();

  const skip = screen.getByRole('button', { name: 'Skip to the page' });
  expect(skip.compareDocumentPosition(sidebar()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  await fireEvent.click(skip);
  expect(document.activeElement).toBe(
    within(page('History')).getByRole('heading', { name: 'History' }),
  );

  await choose('Speaking');
  await waitFor(() => expect(pageHeading('Speaking')).toBeTruthy());
  (document.activeElement as HTMLElement).blur();

  await fireEvent.click(skip);
  expect(document.activeElement).toBe(pageHeading('Speaking'));
});

it('states a failure as one sentence rather than two run together', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  const { container } = render(App);
  await listed();
  daemon.update((s) => ({ ...s, live: { ...s.live, last_error: 'the key was refused' } }));

  await waitFor(() =>
    expect(container.querySelector('.failure p')?.textContent?.trim()).toBe(
      'The last dictation failed: the key was refused.',
    ),
  );
});

// jsdom has no view transitions, so every other test takes the synchronous
// path. The platform runs the callback in a later task, and focus is moved on
// the line after the swap.
it('lands focus on the page even where the platform defers the swap', async () => {
  const deferred = (run: () => void) => {
    const updateCallbackDone = new Promise<void>((resolve) => {
      setTimeout(() => {
        run();
        resolve();
      }, 0);
    });
    return { updateCallbackDone, finished: updateCallbackDone, ready: updateCallbackDone };
  };
  Object.defineProperty(document, 'startViewTransition', {
    configurable: true,
    writable: true,
    value: deferred,
  });
  try {
    vi.mocked(status).mockResolvedValue(ready);
    render(App);
    await listed();
    daemon.update((s) => ({ ...s, live: { ...s.live, last_error: 'the key was refused' } }));

    const open = await screen.findByRole('button', { name: 'Open Listening' });
    await fireEvent.click(open);

    await waitFor(() => expect(document.activeElement?.id).toBe(DICTATION_FAILURE));
  } finally {
    delete (document as Partial<Document>).startViewTransition;
  }
});

// `stt_model` and `stt_translates` arrive only with a full status, and the read a
// write does happens before the file has landed, so without this the window
// keeps reporting the model it was told about at the moment of the write.
it('reads the daemon again when a model finishes loading', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();

  pushes.get('daemon:state')?.({ payload: { loading_model: true } });
  const readsBefore = vi.mocked(status).mock.calls.length;
  pushes.get('daemon:state')?.({ payload: { loading_model: false } });

  await waitFor(() => expect(vi.mocked(status).mock.calls.length).toBeGreaterThan(readsBefore));
});

it('does not re-read the daemon when no model was loading', async () => {
  vi.mocked(status).mockResolvedValue(ready);
  render(App);
  await listed();

  const readsBefore = vi.mocked(status).mock.calls.length;
  pushes.get('daemon:state')?.({ payload: { loading_model: false } });
  await Promise.resolve();

  expect(vi.mocked(status).mock.calls.length).toBe(readsBefore);
});

// A status read is the other way the window learns a load is running, and the
// falling edge is judged against what it held, not against the push alone.
it('follows up a load it learned from a status read rather than a push', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, loading_model: true });
  render(App);
  await listed();

  const readsBefore = vi.mocked(status).mock.calls.length;
  pushes.get('daemon:state')?.({ payload: { loading_model: false } });

  await waitFor(() => expect(vi.mocked(status).mock.calls.length).toBeGreaterThan(readsBefore));
});

// A grant made in System Settings, a hand-edited config.toml and a model file
// added outside Banshee reach no push. Returning to the window is what catches
// them, and the reader is always coming back from another application.
it('reads the daemon again when the window is returned to', async () => {
  render(App);
  await listed();

  const readsBefore = vi.mocked(status).mock.calls.length;
  await fireEvent.focus(window);

  await waitFor(() => expect(vi.mocked(status).mock.calls.length).toBeGreaterThan(readsBefore));
});

it('does not start the daemon when the window is returned to', async () => {
  vi.mocked(status).mockRejectedValue(new Error('not running'));
  render(App);
  await waitFor(() => expect(screen.getByText('Banshee is not running')).toBeTruthy());

  vi.mocked(startDaemon).mockClear();
  await fireEvent.focus(window);
  await waitFor(() => expect(vi.mocked(status).mock.calls.length).toBeGreaterThan(0));

  expect(vi.mocked(startDaemon)).not.toHaveBeenCalled();
});

// A window unhidden without the pointer landing in it fires no focus event.
it('reads the daemon again when the window becomes visible', async () => {
  render(App);
  await listed();

  const readsBefore = vi.mocked(status).mock.calls.length;
  document.dispatchEvent(new Event('visibilitychange'));

  await waitFor(() => expect(vi.mocked(status).mock.calls.length).toBeGreaterThan(readsBefore));
});

// `visibilitychange` fires on the way out as well as the way back, and a window
// being hidden has nobody to show a fresh reading to.
it('does not read the daemon when the window is hidden', async () => {
  render(App);
  await listed();

  const readsBefore = vi.mocked(status).mock.calls.length;
  const visible = Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState');
  Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
  try {
    document.dispatchEvent(new Event('visibilitychange'));
    await Promise.resolve();
    expect(vi.mocked(status).mock.calls.length).toBe(readsBefore);
  } finally {
    if (visible) Object.defineProperty(document, 'visibilityState', visible);
  }
});

it('releases the download controls once the run reports', async () => {
  vi.mocked(downloadModels).mockResolvedValue(undefined);
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [
      {
        kind: 'model',
        id: 'ggml-x.bin',
        name: 'ggml-x.bin',
        role: 'speech',
        remedy: 'download',
        consequence: 'recording does not work',
        fix: 'run: banshee setup',
        command: 'banshee setup',
      },
    ] as Blocker[],
  });
  render(App);
  const start = await screen.findByRole('button', { name: /Download/ });
  await fireEvent.click(start);

  await waitFor(() =>
    expect((screen.getByRole('button', { name: /Download/ }) as HTMLButtonElement).disabled).toBe(
      true,
    ),
  );

  pushes.get('daemon:downloads')?.({
    payload: { model: 'ggml-x.bin', bytes: 10, total: 100, state: 'done' },
  });

  await waitFor(() =>
    expect((screen.getByRole('button', { name: /Download/ }) as HTMLButtonElement).disabled).toBe(
      false,
    ),
  );
});

describe('below the width the sidebar folds at', () => {
  const WIDE = `(min-width: ${SIDEBAR_FOLDS_BELOW}px)`;
  let widen: (query: string, matches: boolean) => void;
  const unfold = () => screen.getByRole('button', { name: 'Show sidebar' });
  const drawer = () => screen.queryByRole('dialog', { name: 'Sidebar' });
  // Whole, because the host sits in its own span. The Listening page's note
  // says the same words with a full stop.
  const hostLine = (_: string, node: Element | null) =>
    node?.textContent === 'Audio goes to api.openai.com';

  beforeEach(() => {
    widen = stubMatchMedia({ [WIDE]: false });
  });
  afterEach(() => vi.unstubAllGlobals());

  it('opens the sidebar from a toolbar button in a modal dialog', async () => {
    const modal = vi.spyOn(HTMLDialogElement.prototype, 'showModal');
    render(App);
    await listed();
    expect(screen.queryByRole('navigation', { name: 'Banshee' })).toBeNull();
    expect(drawer()).toBeNull();

    await fireEvent.click(unfold());
    expect(modal).toHaveBeenCalled();
    expect(within(drawer()!).getByRole('navigation', { name: 'Banshee' })).toBeTruthy();
    expect(within(page('History')).getByRole('button', { name: 'Show sidebar' })).toBe(unfold());

    await fireEvent.keyDown(item('Agents'), { key: 'Escape' });
    expect(drawer()).toBeNull();
    expect(document.activeElement).toBe(unfold());

    await fireEvent.click(unfold());
    await choose('Hotkey');
    await waitFor(() => expect(page('Hotkey')).toBeTruthy());
    expect(drawer()).toBeNull();
    expect(document.activeElement).toBe(unfold());
    modal.mockRestore();
  });

  it('carries no accessibility violations with the sidebar open', async () => {
    const { container } = render(App);
    await listed();
    await fireEvent.click(unfold());
    expect(drawer()).toBeTruthy();
    const results = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } });
    expect(results.violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`)).toEqual([]);
  });

  it('names the host above the page content', async () => {
    vi.mocked(status).mockResolvedValue(remote);
    render(App);
    await listed();

    const line = within(page('History')).getByText(hostLine);
    expect(
      line.compareDocumentPosition(screen.getByText('Yes, open the pull request.')) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    await fireEvent.click(unfold());
    await choose('Listening');
    await waitFor(() => expect(page('Listening')).toBeTruthy());
    expect(
      within(page('Listening'))
        .getByText(hostLine)
        .compareDocumentPosition(pageHeading('Listening')) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    widen(WIDE, true);
    await waitFor(() => expect(within(page('Listening')).queryByText(hostLine)).toBeNull());
    expect(within(sidebar()).getByText(/Audio goes to/)).toBeTruthy();
  });

  it('reveals search from a toolbar icon, and Escape puts it away again', async () => {
    render(App);
    await listed();
    expect(screen.queryByRole('searchbox')).toBeNull();

    const reveal = screen.getByRole('button', { name: 'Search' });
    await fireEvent.click(reveal);
    const find = await screen.findByRole('searchbox', { name: 'Search' });
    await waitFor(() => expect(document.activeElement).toBe(find));

    await fireEvent.input(find, { target: { value: 'older' } });
    await fireEvent.keyDown(find, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('searchbox')).toBeNull());
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Search' }));
    expect(screen.getAllByRole('option').length).toBe(2);
  });

  it('leaves the page behind the open sidebar to its own keys', async () => {
    render(App);
    await listed();
    await fireEvent.click(unfold());

    await fireEvent.keyDown(item('Agents'), { key: 'f', metaKey: true, ctrlKey: true });
    await fireEvent.keyDown(item('Agents'), { key: 'c', metaKey: true, ctrlKey: true });
    expect(drawer()).toBeTruthy();
    expect(screen.queryByRole('searchbox')).toBeNull();
    expect(copyText).not.toHaveBeenCalled();
  });

  // One key, one effect: the Escape that closes the dialog also reaches the window.
  it('leaves the search as it was when Escape closes the sidebar', async () => {
    render(App);
    await listed();
    await fireEvent.keyDown(window, { key: 'f', metaKey: true });
    const find = await screen.findByRole('searchbox', { name: 'Search' });
    await fireEvent.input(find, { target: { value: 'older' } });

    await fireEvent.click(unfold());
    await fireEvent.keyDown(item('Agents'), { key: 'Escape' });
    expect(drawer()).toBeNull();
    expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('older');
    expect(screen.getAllByRole('option').length).toBe(1);
  });

  it('puts Back in the toolbar while a row is open, and it returns to the same option', async () => {
    render(App);
    await listed();
    expect(screen.queryByRole('button', { name: 'Back to History' })).toBeNull();

    await fireEvent.click(screen.getByRole('option', { name: /^the older one,/ }));
    const back = await screen.findByRole('button', { name: 'Back to History' });
    const heading = within(page('History')).getByRole('heading', { name: 'History' });
    expect(heading.parentElement!.contains(back)).toBe(true);
    expect(screen.getByRole('region', { name: 'Selected row' }).contains(back)).toBe(false);

    await fireEvent.click(back);
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole('option', { name: /^the older one,/ })),
    );
    expect(screen.queryByRole('region', { name: 'Selected row' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Back to History' })).toBeNull();
  });

  it('lands focus on the detail heading when Back goes as the window widens', async () => {
    render(App);
    await listed();
    await fireEvent.click(screen.getByRole('option', { name: /^the older one,/ }));
    const back = await screen.findByRole('button', { name: 'Back to History' });
    back.focus();

    widen(`(min-width: ${LIST_AND_DETAIL_FROM}px)`, true);
    await waitFor(() =>
      expect(document.activeElement).toBe(
        within(screen.getByRole('region', { name: 'Selected row' })).getByRole('heading'),
      ),
    );
  });

  it('keeps focus in the sidebar or on its button as the window crosses the width', async () => {
    render(App);
    await listed();
    await fireEvent.click(unfold());
    await choose('Feedback');
    await waitFor(() => expect(page('Feedback')).toBeTruthy());
    await fireEvent.click(unfold());

    widen(WIDE, true);
    await waitFor(() => expect(drawer()).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(item('Feedback')));
    expect(item('Feedback').getAttribute('aria-current')).toBe('page');
    expect(screen.queryByRole('button', { name: 'Show sidebar' })).toBeNull();

    widen(WIDE, false);
    await waitFor(() => expect(document.activeElement).toBe(unfold()));
  });
});
