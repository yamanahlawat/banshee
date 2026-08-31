import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import ready from './fixtures/ready.json';
import permissions from './fixtures/permissions.json';

// Local noon, because the history helpers read the local calendar day.
const NOW = new Date(2026, 7, 27, 12, 0, 0);

function stamp(minutesAgo: number): string {
  return `${new Date(NOW.getTime() - minutesAgo * 60_000).toISOString().slice(0, 19)}Z`;
}

// The daemon answers oldest first, so the newest row is last.
const rows = [
  { id: 1, text: 'the older one', timestamp: stamp(9) },
  { id: 2, text: 'Yes, open the pull request.', timestamp: stamp(1) },
];

vi.mock('./lib/tauri', async () => (await import('./lib/tauri.mock')).mockTauri());

import {
  detectAgents,
  history,
  listen,
  listLanguages,
  listVoices,
  listDevices,
  downloadModels,
  openPermissionPane,
  restartDaemon,
  setSetting,
  startDaemon,
  status,
} from './lib/tauri';
import { agents } from './lib/agents';
import { table as historyTable } from './lib/history';
import { daemon, empty, reduceStatus } from './lib/daemon';
import { forgetCopy } from './lib/copy';
import { forgetKeys } from './lib/keys';
import App from './App.svelte';

// A panel's heading is its lead statement now, and that sentence moves with
// the daemon. The section keeps the short name, so tests ask for that and read
// the heading inside it.
const panel = (name: string) => screen.getByRole('region', { name });
const panelHeading = (name: string) => within(panel(name)).getByRole('heading');

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
  forgetCopy();
  forgetKeys();
  vi.mocked(status).mockResolvedValue(ready);
  vi.mocked(history).mockResolvedValue(rows);
  pushes.clear();
  vi.mocked(listen).mockImplementation((name: string, handler: unknown) => {
    pushes.set(name, handler as (event: { payload: unknown }) => void);
    return Promise.resolve(() => {});
  });
  vi.mocked(listDevices).mockResolvedValue({ devices: [], current: null });
  vi.mocked(listLanguages).mockResolvedValue({
    languages: [
      { code: 'en', name: 'English' },
      { code: 'hi', name: 'Hindi' },
    ],
  });
  vi.mocked(listVoices).mockResolvedValue({
    voices: [{ id: 'af_sky', name: 'Sky', description: 'American, clear' }],
    current: 'af_sky',
  });
  vi.mocked(detectAgents).mockResolvedValue([]);
});

it('names the speaker in words, because the type cut says nothing to a screen reader', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  expect(screen.getAllByText(/^You said at /).length).toBe(rows.length);
});

it('sets the newest turn apart from the ones below it', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  const turns = container.querySelectorAll('article.turn');
  expect(turns.length).toBe(rows.length);
  expect(turns[0].classList.contains('lead')).toBe(true);
  expect(turns[1].classList.contains('lead')).toBe(false);
});

it('draws an empty history rather than leaving the body blank', async () => {
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() => expect(screen.getByText('Nothing said yet')).toBeTruthy());
});

it('says what a blocker stops and offers the pane that clears it', async () => {
  vi.mocked(status).mockResolvedValue(permissions);
  render(App);
  const open = await screen.findAllByRole('button', { name: /^Open System Settings for / });
  await fireEvent.click(open[0]);
  expect(vi.mocked(openPermissionPane)).toHaveBeenCalled();
});

it('shows that an agent is waiting, without claiming to know the question', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  daemon.update((s) => ({ ...s, live: { ...s.live, armed: true } }));
  await waitFor(() =>
    expect(screen.getByText('An agent asked a question and is waiting for your answer.')).toBeTruthy(),
  );
});

it('opens a job into the body and closes it again', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  await fireEvent.click(screen.getByRole('button', { name: /Hotkey/ }));
  expect(screen.getByRole('button', { name: 'Done' })).toBeTruthy();
  expect(screen.queryByText('Yes, open the pull request.')).toBeNull();

  await fireEvent.click(screen.getByRole('button', { name: 'Done' }));
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
});

it('asks before it deletes the record', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  // Reachable from the head of the list, because the foot of a long list is not.
  await fireEvent.click(screen.getByRole('button', { name: /saved/ }));
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  expect(screen.getByText(/This cannot be undone/)).toBeTruthy();
});

it('renders a page of turns at a time rather than the whole record', async () => {
  const many = Array.from({ length: 95 }, (_, i) => ({
    id: i + 1,
    text: `dictation ${i + 1}`,
    timestamp: stamp(95 - i),
  }));
  vi.mocked(history).mockResolvedValue(many);

  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('dictation 95')).toBeTruthy());

  expect(container.querySelectorAll('article.turn').length).toBe(40);
  await fireEvent.click(screen.getByRole('button', { name: '55 older' }));
  expect(container.querySelectorAll('article.turn').length).toBe(80);
});

it('filters what was said instead of opening a second place for it', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  expect(screen.queryByRole('searchbox')).toBeNull();
  await fireEvent.keyDown(window, { key: 'f', metaKey: true });

  const find = screen.getByRole('searchbox', { name: 'Find in what was said' });
  await fireEvent.input(find, { target: { value: 'older' } });

  await waitFor(() => expect(screen.getByText('the older one')).toBeTruthy());
  expect(screen.queryByText('Yes, open the pull request.')).toBeNull();

  await fireEvent.input(find, { target: { value: 'nothing matches this' } });
  await waitFor(() => expect(screen.getByText('No match')).toBeTruthy());

  await fireEvent.keyDown(window, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('searchbox')).toBeNull());
  expect(screen.getByText('Yes, open the pull request.')).toBeTruthy();
});

it('says what Banshee does with your words without being asked', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  expect(screen.getByRole('button', { name: 'Stop saving' })).toBeTruthy();
  expect(screen.getByRole('button', { name: /saved/ })).toBeTruthy();
});

it('marks a foot value the daemon has not taken', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['audio.hotkey'] });
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  // The daemon reports no bound key, so the foot names the configured one. The
  // mark is the only thing that stops it reading as the key in force.
  //
  // The class carries the dashed rule. jsdom resolves no scoped stylesheet, so
  // the class is the only part of the visual mark a test here can see.
  expect(screen.getByText('Right Command').classList.contains('pending')).toBe(true);
  expect(screen.getByText('— set, and in effect when Banshee restarts')).toBeTruthy();
});

it('leaves a foot value unmarked when the daemon has taken it', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  expect(screen.getByText('Right Command').classList.contains('pending')).toBe(false);
  expect(screen.queryByText('— set, and in effect when Banshee restarts')).toBeNull();
});

it('marks the saving switch the daemon has not taken', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['daemon.save_history'] });
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  // The switch carries the mark, and the find hint keeps its own place.
  const swtch = screen.getByRole('button', {
    name: /Stop saving.*in effect when Banshee restarts/,
  });
  expect(swtch.classList.contains('pending')).toBe(true);
});

it('restarts from the header rather than naming a command', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['audio.hotkey'] });
  render(App);

  // `audio.hotkey` is never applied live, so the only way out is a new process.
  // The window can do that, so the notice is the control.
  const notice = await screen.findByRole('button', { name: 'Restart to apply' });
  await fireEvent.click(notice);
  expect(vi.mocked(restartDaemon)).toHaveBeenCalled();
});

it('asks for a restart in the header, which a panel does not cover', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['audio.hotkey'] });
  render(App);
  await waitFor(() => expect(screen.getByText('Restart to apply')).toBeTruthy());

  // The ledger scrolls away and a job takes the body; the header does neither.
  await fireEvent.click(screen.getByRole('button', { name: /Hotkey/ }));
  await waitFor(() => expect(panelHeading('Hotkey')).toBeTruthy());
  expect(screen.getByText('Restart to apply')).toBeTruthy();
});

it('asks for no restart when the daemon has taken everything', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  expect(screen.queryByText('Restart to apply')).toBeNull();
});

it('follows what the daemon says about history, not what the config asked for', async () => {
  // The two disagree when the history file will not open: the write landed in
  // the config and the daemon kept its old connection.
  vi.mocked(status).mockResolvedValue({ ...ready, history_enabled: false });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Start saving' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Stop saving' })).toBeNull();
});

it('holds the place words will take while the microphone is open', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  daemon.update((s) => ({ ...s, live: { ...s.live, recording: true } }));
  await waitFor(() =>
    expect(screen.getByText('Recording. What you say will appear here.')).toBeTruthy(),
  );
  // The sentence is the row's own text, not something only a screen reader is
  // given. A 3px bar said nothing to a reader, and told recording and working
  // apart by hue alone.
  const pending = container.querySelector('.turn');
  expect(pending?.querySelector('.sr')).toBeNull();
  expect(pending?.querySelector('.caret')).toBeNull();

  daemon.update((s) => ({ ...s, live: { ...s.live, recording: false, transcribing: true } }));
  await waitFor(() => expect(screen.getByText('Working out what you said.')).toBeTruthy());

  // Speaking produces no turn, so it must not hold a place for one.
  daemon.update((s) => ({ ...s, live: { ...s.live, transcribing: false, speaking: true } }));
  await waitFor(() =>
    expect(screen.queryByText('Working out what you said.')).toBeNull(),
  );
});

it('offers a way back when the daemon has stopped', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  daemon.update((s) => ({ ...s, down: 'not running' }));
  await waitFor(() => expect(screen.getByText('Banshee is not running')).toBeTruthy());
  expect(screen.getByRole('button', { name: 'Start Banshee' })).toBeTruthy();
  // What was said before is still readable.
  expect(screen.getByText('Yes, open the pull request.')).toBeTruthy();
});

it('does not reflow the paragraph when a copy is confirmed', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  const named = screen.getAllByRole('button', { name: /^Copy what you said at \d\d:\d\d$/ });
  expect(named.length).toBeGreaterThan(1);
  const copy = named[0];
  // NOT COVERED HERE: that the box keeps its width between the two labels.
  // jsdom resolves no scoped stylesheet and has no layout engine, so
  // `getComputedStyle` answers `auto` with the reservation deleted. The
  // reservation is `width: 68px` in Turn.svelte, verified against a real render.
  await fireEvent.click(copy);
  await waitFor(() => expect(screen.getAllByText('Copied').length).toBeGreaterThan(0));
  // What this can prove: confirming one turn's copy does not touch another's.
  const others = container.querySelectorAll('article.turn button');
  expect([...others].filter((b) => b.textContent?.includes('Copied')).length).toBe(1);
});

it('keeps the newest turn monumental while the microphone is open', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  daemon.update((s) => ({ ...s, live: { ...s.live, recording: true } }));
  await waitFor(() =>
    expect(screen.getByText('Recording. What you say will appear here.')).toBeTruthy(),
  );

  const turns = container.querySelectorAll('article.turn');
  const leads = container.querySelectorAll('article.turn.lead');
  expect(leads.length).toBe(1);
  expect(leads[0].textContent).toContain('Yes, open the pull request.');
  expect(turns[0].classList.contains('lead')).toBe(false);
});

it('subscribes to every push the daemon sends before reading it', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  const events = vi.mocked(listen).mock.calls.map((c) => c[0]);
  for (const name of ['daemon:status', 'daemon:state', 'daemon:downloads', 'daemon:down', 'app:reopened']) {
    expect(events).toContain(name);
  }
});

it('starts the daemon itself when it finds it gone', async () => {
  vi.mocked(status).mockRejectedValue({ message: 'not running' });
  render(App);
  await waitFor(() => expect(vi.mocked(startDaemon)).toHaveBeenCalled());
  await waitFor(() => expect(screen.getByText('Banshee is not running')).toBeTruthy());
});

it('reads the voices and the agents the strip needs', async () => {
  render(App);
  await waitFor(() => expect(vi.mocked(listVoices)).toHaveBeenCalled());
  await waitFor(() => expect(vi.mocked(detectAgents)).toHaveBeenCalled());
  // Neither read may take the status and the history down with it.
  expect(screen.getByText('Yes, open the pull request.')).toBeTruthy();
});

it('empties the record when the daemon says it is no longer saving', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  // The daemon reports both once it takes the switch.
  const off = {
    ...ready,
    history_enabled: false,
    config: { ...ready.config, daemon: { save_history: false } },
  };
  daemon.update((s) => reduceStatus(s, off));
  // The daemon refuses the read outright while this is off.
  await waitFor(() => expect(screen.getByText('Nothing is kept')).toBeTruthy());
  expect(screen.queryByText('Yes, open the pull request.')).toBeNull();
});

it('asks which speech model to fetch before it fetches one', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, ready: false, download_megabytes: 860, blockers: [{ kind: 'model', id: 'ggml-large-v3-turbo-q5_0.bin', name: 'ggml-large-v3-turbo-q5_0.bin', role: 'speech', remedy: 'download', consequence: 'recording, dictation, and ask_user do not work', fix: 'run: banshee setup', command: 'banshee setup' }] });
  render(App);

  // The preset decides what the run costs, and nothing else asks. The daemon
  // answers with what is still missing rather than a fixed total.
  await waitFor(() => expect(screen.getByRole('radiogroup', { name: 'Speech model' })).toBeTruthy());
  expect(screen.getByRole('radio', { name: 'Balanced' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByText(/About 860 MB to fetch/)).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy();

  // The daemon blocks on two files and the run fetches four, so a count here
  // contradicts the `1 of 4` the progress line shows a moment later.
  expect(screen.getByText('Banshee needs its models')).toBeTruthy();
  expect(screen.queryByText(/files are missing|file is missing/)).toBeNull();
});

it('offers a restart, not a download, when the files are already there', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, ready: false, blockers: [{ kind: 'model', id: 'recording_pipeline', name: 'Recording pipeline', remedy: 'restart', consequence: 'a model would not load', fix: 'restart it: banshee start', command: 'banshee start' }] });
  render(App);

  // The daemon calls this a model fault, but its fix is `banshee start`. Routing
  // on the kind offered Download for files that are on disk.
  await waitFor(() => expect(screen.getByRole('button', { name: 'Restart Banshee' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();

  // Every file is on disk in this state, so counting it as a missing file lies.
  expect(screen.getByText('Banshee needs a restart')).toBeTruthy();
  expect(screen.queryByText(/file is missing|files are missing/)).toBeNull();
  expect(screen.queryByRole('radiogroup', { name: 'Speech model' })).toBeNull();

  // `startDaemon` runs `launchctl kickstart`, which leaves a running job alone.
  // This blocker is offered while the daemon runs, so only a restart clears it.
  vi.mocked(status).mockResolvedValue({ ...ready, blockers: [] });
  await fireEvent.click(screen.getByRole('button', { name: 'Restart Banshee' }));
  expect(vi.mocked(restartDaemon)).toHaveBeenCalled();
  expect(vi.mocked(startDaemon)).not.toHaveBeenCalled();

  // Nothing pushes a blocker change, so a restart that does not re-read leaves
  // the box asking for the restart it has just been given.
  await waitFor(() =>
    expect(screen.queryByRole('button', { name: 'Restart Banshee' })).toBeNull(),
  );
});

it('says which file is downloading and how far it has come', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, ready: false, blockers: [{ kind: 'model', id: 'ggml-large-v3-turbo-q5_0.bin', name: 'ggml-large-v3-turbo-q5_0.bin', role: 'speech', remedy: 'download', consequence: 'recording, dictation, and ask_user do not work', fix: 'run: banshee setup', command: 'banshee setup' }] });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());

  pushes.get('daemon:downloads')?.({
    payload: { model: 'ggml.bin', label: 'Speech model', index: 1, count: 4, bytes: 40, total: 100, state: 'downloading' },
  });
  await waitFor(() => expect(screen.getByText('Speech model, 1 of 4 · 40%')).toBeTruthy());

  // The two blocking files land before the other two, so the daemon starts
  // asking for a restart while the run is still going. One thing is happening,
  // and the box says which.
  expect(screen.getByText(/Getting Banshee's models/)).toBeTruthy();
  expect(screen.queryByRole('radiogroup', { name: 'Speech model' })).toBeNull();

  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [{ kind: 'model', id: 'recording_pipeline', name: 'Recording pipeline', remedy: 'restart', consequence: 'a model would not load', fix: 'restart it: banshee start', command: 'banshee start' }],
  });
  daemon.update((state) => reduceStatus(state, { ...ready, running: true, blockers: [{ kind: 'model', id: 'recording_pipeline', name: 'Recording pipeline', remedy: 'restart', consequence: 'a model would not load', fix: 'restart it: banshee start', command: 'banshee start' }] }));
  await waitFor(() => expect(screen.getByText('Speech model, 1 of 4 · 40%')).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Restart Banshee' })).toBeNull();
});

it('re-reads the daemon when a download ends, so the box stops asking', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, ready: false, blockers: [{ kind: 'model', id: 'ggml-large-v3-turbo-q5_0.bin', name: 'ggml-large-v3-turbo-q5_0.bin', role: 'speech', remedy: 'download', consequence: 'recording, dictation, and ask_user do not work', fix: 'run: banshee setup', command: 'banshee setup' }] });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());

  // Files landing changes what the daemon is blocked on, and no other push
  // says so.
  vi.mocked(status).mockResolvedValue({ ...ready, blockers: [] });
  pushes.get('daemon:downloads')?.({ payload: { model: 'ggml.bin', bytes: 100, total: 100, state: 'done' } });
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Download' })).toBeNull());
});

it('offers nothing to stop before anything has been said', async () => {
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() => expect(screen.getByText('Nothing said yet')).toBeTruthy());

  // The ledger and the absence would state one emptiness twice, and the switch
  // would offer to stop a thing that has never started.
  expect(screen.queryByRole('button', { name: 'Stop saving' })).toBeNull();
  expect(screen.queryByText(/Nothing saved yet/)).toBeNull();
});

it('still says so when it is keeping nothing, which is the claim', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, history_enabled: false });
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Start saving' })).toBeTruthy());
});

it('offers no command to copy, because every fix is a button', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [{ kind: 'model', id: 'ggml.bin', name: 'ggml.bin', role: 'speech', remedy: 'download', consequence: 'x', fix: 'run: banshee setup', command: 'banshee setup' }],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'banshee setup' })).toBeNull();
});

// Choosing a preset marks it pending at once, because the daemon cannot load a
// model that is not on disk. A restart reads the same config and fails the same
// way, so the notice would be advice that cannot work.
it('does not ask for a restart while the files it needs are still missing', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    pending: ['stt.preset'],
    blockers: [{ kind: 'model', id: 'ggml-large-v3-turbo-q5_0.bin', name: 'ggml-large-v3-turbo-q5_0.bin', role: 'speech', remedy: 'download', consequence: 'recording, dictation, and ask_user do not work', fix: 'run: banshee setup', command: 'banshee setup' }],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Restart to apply' })).toBeNull();
});

it('asks for a restart once the files are there and a setting still waits', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['stt.preset'] });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Restart to apply' })).toBeTruthy());
});

// The voice files arrive with the download, and the last one a run fetches is a
// voice, so the list read at startup is answered before any of them exist.
it('reads the voices again once their files have landed', async () => {
  vi.mocked(listVoices).mockResolvedValue({ voices: [], current: null });
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [{ kind: 'model', id: 'ggml.bin', name: 'ggml.bin', role: 'speech', remedy: 'download', consequence: 'x', fix: 'run: banshee setup', command: 'banshee setup' }],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
  expect(screen.getByText('af_sky')).toBeTruthy();

  vi.mocked(status).mockResolvedValue({ ...ready, blockers: [] });
  vi.mocked(listVoices).mockResolvedValue({
    voices: [{ id: 'af_sky', name: 'Sky', description: 'American, clear' }],
    current: 'af_sky',
  });
  pushes.get('daemon:downloads')?.({
    payload: { model: 'af_sky.bin', label: 'Voice', index: 4, count: 4, bytes: 1, total: 1, state: 'done' },
  });

  // The foot names the voice, rather than the file the daemon fetched.
  await waitFor(() => expect(screen.getByText('Sky')).toBeTruthy());
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
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /Voice/ }));

  await waitFor(() => expect(panelHeading('Voice')).toBeTruthy());
  expect(screen.getByText('George')).toBeTruthy();

  // Nothing to play until the file is here.
  expect(screen.getByRole('button', { name: 'Preview George' }).hasAttribute('disabled')).toBe(true);
  expect(screen.getByRole('button', { name: 'Preview Sky' }).hasAttribute('disabled')).toBe(false);

  await fireEvent.change(screen.getByRole('radio', { name: /George/ }));
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
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /Voice/ }));
  await waitFor(() => expect(panelHeading('Voice')).toBeTruthy());

  await fireEvent.change(screen.getByRole('radio', { name: /Sky/ }));
  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.voice', 'af_sky'));
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
});

// Choosing a voice that is not here writes it, and the daemon refuses until the
// file lands, so `tts.voice` is pending for the second the fetch takes. A
// restart is not the remedy: the daemon applies it when the run ends.
it('does not ask for a restart while the file it waits on is still arriving', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, pending: ['tts.voice'] });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Restart to apply' })).toBeTruthy());

  pushes.get('daemon:downloads')?.({
    payload: { model: 'bm_george.bin', label: 'Voice', index: 1, count: 1, bytes: 1, total: 2, state: 'downloading' },
  });
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Restart to apply' })).toBeNull());
});

// A run reports once per file. A download writes no history, so the record
// cannot have moved and only the status is worth asking for again.
it('does not re-read the record for every file a download finishes', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [{ kind: 'model', id: 'ggml.bin', name: 'ggml.bin', role: 'speech', remedy: 'download', consequence: 'x', fix: 'run: banshee setup', command: 'banshee setup' }],
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
      { kind: 'permission', id: 'input_monitoring', name: 'Input Monitoring', consequence: 'the hotkey receives no key presses', fix: 'grant it in System Settings' },
      { kind: 'model', id: 'ggml.bin', name: 'ggml.bin', role: 'speech', remedy: 'download', consequence: 'x', fix: 'run: banshee setup', command: 'banshee setup' },
    ],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: /^Open System Settings for / })).toBeTruthy());

  pushes.get('daemon:downloads')?.({
    payload: { model: 'ggml.bin', label: 'Speech model', index: 1, count: 4, bytes: 1, total: 2, state: 'downloading' },
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
    blockers: [{ kind: 'model', id: 'ggml.bin', name: 'ggml.bin', role: 'speech', remedy: 'download', consequence: 'x', fix: 'run: banshee setup', command: 'banshee setup' }],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());

  await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
  await waitFor(() => expect(screen.getByText(/Download did not work/)).toBeTruthy());
});

// `audio.hotkey` is read once at startup and no arriving file answers it, so a
// download must not quiet the notice that says so.
it('still asks for a restart for a key no download can settle', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    pending: ['audio.hotkey'],
    blockers: [{ kind: 'model', id: 'ggml.bin', name: 'ggml.bin', role: 'speech', remedy: 'download', consequence: 'x', fix: 'run: banshee setup', command: 'banshee setup' }],
  });
  render(App);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Restart to apply' })).toBeTruthy());
});

// Whisper translates one way only, any language in and English out, so the
// answer control means nothing until a language other than English is set.
it('offers a language, and asks what to answer in only once it can mean something', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: { ...ready.config, stt: { ...ready.config.stt, preset: 'balanced', language: 'en' } },
  });
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));
  await waitFor(() => expect(screen.getByRole('combobox', { name: 'Language' })).toBeTruthy());

  expect(screen.queryByRole('radiogroup', { name: 'Answer in' })).toBeNull();

  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: { ...ready.config, stt: { ...ready.config.stt, preset: 'balanced', language: 'hi' } },
  });
  daemon.update((state) => reduceStatus(state, {
    ...ready,
    running: true,
    config: { ...ready.config, stt: { ...ready.config.stt, preset: 'balanced', language: 'hi' } },
  }));
  await waitFor(() => expect(screen.getByRole('radiogroup', { name: 'Answer in' })).toBeTruthy());
});

// The English-only build holds no other language, so a control that looked
// live would quietly do nothing.
it('will not offer a language the fast model cannot hear', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    english_only: true,
    config: { ...ready.config, stt: { ...ready.config.stt, preset: 'fast', language: 'en' } },
  });
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));

  const picker = await screen.findByRole('combobox', { name: 'Language' });
  expect(picker.hasAttribute('disabled')).toBe(true);
  expect(screen.getByText(/Fast hears English only/)).toBeTruthy();
});

// The ledger is the only route to the Record panel and it stands down while the
// record is empty, so a fresh install could not reach the saving switch at all.
it('reaches what Banshee keeps before anything has been said', async () => {
  vi.mocked(history).mockResolvedValue([]);
  render(App);
  await waitFor(() => expect(screen.getByText('Nothing said yet')).toBeTruthy());

  await fireEvent.click(screen.getByRole('button', { name: 'What Banshee keeps' }));
  await waitFor(() => expect(panelHeading('What Banshee keeps')).toBeTruthy());
  expect(screen.getByRole('button', { name: /Stop saving/ })).toBeTruthy();
});

// A microphone fault and a model fault carry the same command, and restarting
// does nothing for a device that is not plugged in.
it('does not call an unplugged microphone a restart', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    ready: false,
    blockers: [{ kind: 'pipeline', id: 'recording_pipeline', name: 'Recording pipeline', remedy: 'restart', consequence: 'the microphone would not open: no device', fix: 'connect the microphone, grant it in Privacy & Security, or fix [audio] input_device. If recording does not recover on its own, restart: banshee start', command: 'banshee start' }],
  });
  render(App);
  await waitFor(() => expect(screen.getByText('The microphone is not working')).toBeTruthy());

  expect(screen.queryByText('Banshee needs a restart')).toBeNull();
  expect(screen.getByRole('button', { name: 'Restart anyway' })).toBeTruthy();
  expect(screen.getByText(/onnect the microphone/)).toBeTruthy();
});

// `auto` is a value the config takes and the engine reads as detect it, so a
// picker that cannot show it misreports the daemon and cannot set it back.
it('offers detection among the languages', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: { ...ready.config, stt: { ...ready.config.stt, preset: 'balanced', language: 'auto' } },
  });
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));

  const picker = await screen.findByRole('combobox', { name: 'Language' });
  expect(screen.getByRole('option', { name: 'Detect it' })).toBeTruthy();
  expect((picker as HTMLSelectElement).value).toBe('auto');
});

// A panel replaces the whole body. The control that opened it is destroyed by
// the click it is answering, so the keyboard is left on a node that is gone.
it('puts the keyboard in a panel when it opens, and back where it came from', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  const opener = screen.getByRole('button', { name: /^Microphone/ });
  opener.focus();
  await fireEvent.click(opener);

  await waitFor(() => expect(panel('Microphone')).toBeTruthy());
  const title = panelHeading('Microphone');
  expect(document.activeElement).toBe(title);

  await fireEvent.click(screen.getByRole('button', { name: 'Done' }));
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: /^Microphone/ })));
});

// The ledger sits in the body it opens over, so the way back is the control's
// name and not the node, which no longer exists.
it('gives the keyboard back to the ledger after the record closes', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  const opener = screen.getByRole('button', { name: /saved/ });
  opener.focus();
  await fireEvent.click(opener);
  await waitFor(() => expect(panel('What Banshee keeps')).toBeTruthy());

  await fireEvent.click(screen.getByRole('button', { name: 'Done' }));
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: /saved/ })));
});

// Escape closes a panel too, and leaves the keyboard in the same place a click
// on Done would.
it('gives the keyboard back when Escape closes a panel', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  const opener = screen.getByRole('button', { name: /^Voice/ });
  opener.focus();
  await fireEvent.click(opener);
  await waitFor(() => expect(panel('Voice')).toBeTruthy());
  expect(document.activeElement).toBe(panelHeading('Voice'));

  await fireEvent.keyDown(window, { key: 'Escape' });
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: /^Voice/ })));
});

// With no heading anywhere, a screen reader has no way to reach the record but
// to walk the whole window.
it('carries a heading for the record', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  expect(screen.getAllByRole('heading').length).toBeGreaterThan(0);
});

// The device picker keeps a row for a device that is not there. The language
// picker had no such guard, so a code Whisper's table does not name left a
// `select` whose value matched no option, which draws as an empty control.
it('keeps a row for a language the daemon did not name', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: { ...ready.config, stt: { ...ready.config.stt, preset: 'balanced', language: 'cy' } },
  });
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));

  const picker = await screen.findByRole('combobox', { name: 'Language' });
  expect((picker as HTMLSelectElement).value).toBe('cy');
});

// A swallowed failure left the same empty control with nothing to explain it.
it('says when the language list did not arrive', async () => {
  vi.mocked(listLanguages).mockRejectedValue(new Error('no such method'));
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));

  await waitFor(() => expect(screen.getByText(/could not list the languages/)).toBeTruthy());
});

// The key sat on the same ink underline the pickers use, so it read as a field
// to type into while being a span.
it('lets the hotkey be changed by the key it names', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /^Hotkey/ }));

  const key = await screen.findByRole('button', { name: /Right Command — change the hotkey/ });
  await fireEvent.click(key);
  await waitFor(() => expect(screen.getByText('Press a key')).toBeTruthy());
});

// The differentiating moment cannot be the window's quietest pixel.
it('gives the waiting agent the lead, and names the key that answers it', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  // Armed arrives as a live push mid-session, after the config is known. Set
  // before mount, the turn draws before the window has been told which key.
  daemon.update((s) => ({ ...s, live: { ...s.live, armed: true } }));
  // hotkey_mode is toggle in the ready fixture, so the verb is Tap, not Hold.
  await waitFor(() => expect(screen.getByText(/Tap Right Command to answer/)).toBeTruthy());

  // Normalized, because the sentence wraps in the source and textContent keeps
  // the newline that HTML itself collapses.
  const lead = container.querySelector('.lead')?.textContent?.replace(/\s+/g, ' ');
  expect(lead).toMatch(/is waiting for your answer/);
  // The newest thing you said stands down while an agent is blocked on you.
  expect(lead).not.toMatch(/Yes, open the pull request/);
});

// Giving an agent a voice is the job nothing else here does. Zero connected
// agents is the state that has to say so.
it('offers the agent job on the home screen when none is connected', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await waitFor(() => expect(screen.getByText(/can speak to you/)).toBeTruthy());

  await fireEvent.click(screen.getByRole('button', { name: /Connect an agent/ }));
  await waitFor(() => expect(panelHeading('Agents')).toBeTruthy());
});

// The foot is the only route to every setting, so it cannot sit behind every
// copy control on the page.
it('makes the foot one tab stop the arrows move inside', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  const cells = [...container.querySelectorAll('footer button')] as HTMLElement[];
  expect(cells.map((c) => c.tabIndex)).toEqual([0, -1, -1, -1]);

  cells[0].focus();
  await fireEvent.keyDown(cells[0], { key: 'ArrowRight' });
  expect(document.activeElement).toBe(cells[1]);
  // Moving inside a toolbar moves the focus and opens nothing.
  expect(screen.queryByRole('button', { name: 'Done' })).toBeNull();
});

// Nobody needs 21 steps of a value the window only ever reads back as one of
// three words. The control shows the band the current float falls in.
it('sets sensitivity by band, and writes a float for it', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /^Microphone/ }));

  const group = await screen.findByRole('radiogroup', { name: 'Sensitivity' });
  expect(group).toBeTruthy();
  // ready.json holds 0.5, which is the middle band.
  expect(screen.getByRole('radio', { name: 'Medium' }).getAttribute('aria-checked')).toBe('true');

  await fireEvent.click(screen.getByRole('radio', { name: 'High' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('stt.vad_threshold', 0.85));
});

// tts.fallback serves no job this audience has, and belongs to the CLI.
it('does not spend the Voice panel on a key that belongs to the CLI', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /^Voice/ }));

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
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /^Agents/ }));

  await waitFor(() => expect(screen.getByText('Claude Code')).toBeTruthy());
  expect(screen.getByText(/also works with/)).toBeTruthy();
  expect(screen.queryByText(/^Not installed$/)).toBeNull();
});

// Keeping is the primary, and the prompt names how much would go.
it('makes keeping the record the primary, and names what would be lost', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /saved/ }));
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));

  expect(screen.getByText(/Delete all 2\?/)).toBeTruthy();
  const buttons = screen.getAllByRole('button').map((b) => b.textContent?.trim());
  expect(buttons.indexOf('Keep it')).toBeLessThan(buttons.indexOf('Delete everything'));
});

// Abandoning a half-typed word must not also throw the reader out of the panel.
// The field claims the keyboard for exactly this, and releasing the claim
// inside the same keydown lets the event reach the window handler unclaimed.
it('keeps the panel open when Escape abandons a vocabulary word', async () => {
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /^Microphone/ }));
  await fireEvent.click(await screen.findByRole('button', { name: 'Add a word' }));

  const field = screen.getByRole('textbox', { name: 'New word' });
  await fireEvent.keyDown(field, { key: 'Escape' });

  expect(screen.queryByRole('region', { name: 'Microphone' })).toBeTruthy();
});

// The way out of a panel cannot sit behind every control in it.
it('reaches Done by tabbing forward from the panel heading', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /^Voice/ }));
  await waitFor(() => expect(panel('Voice')).toBeTruthy());

  const order = [...container.querySelectorAll('.panel button, .panel input, .panel select')];
  const heading = panelHeading('Voice');
  const done = screen.getByRole('button', { name: 'Done' });
  // Focus lands on the heading, so Done has to come after it in the order the
  // keyboard walks, not before.
  expect(heading.compareDocumentPosition(done) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(order[0]).toBe(done);
});

// Tab out and back should return where the arrows left, which is what a roving
// tab stop is for.
it('moves the foot tab stop with the arrows', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  const cells = [...container.querySelectorAll('footer button')] as HTMLElement[];
  cells[0].focus();
  await fireEvent.keyDown(cells[0], { key: 'ArrowRight' });
  expect(cells.map((c) => c.tabIndex)).toEqual([-1, 0, -1, -1]);
});

// A hand-edited config, or one written by a newer daemon, holds values these
// options do not. Every cell at -1 puts the group out of the keyboard's reach.
it('keeps a tab stop when no option matches the daemon', async () => {
  vi.mocked(status).mockResolvedValue({
    ...ready,
    config: { ...ready.config, audio: { ...ready.config.audio, barge_in: 'something-else' } },
  });
  render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: /^Hotkey/ }));

  const group = await screen.findByRole('radiogroup', { name: 'While Banshee is talking' });
  const stops = [...group.querySelectorAll('button')].map((b) => (b as HTMLElement).tabIndex);
  expect(stops).toContain(0);
});
