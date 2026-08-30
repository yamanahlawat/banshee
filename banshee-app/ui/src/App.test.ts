import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
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
  listVoices,
  listDevices,
  openPermissionPane,
  startDaemon,
  status,
} from './lib/tauri';
import { agents } from './lib/agents';
import { table as historyTable } from './lib/history';
import { daemon, empty, reduceStatus } from './lib/daemon';
import { forgetCopy } from './lib/copy';
import { forgetKeys } from './lib/keys';
import App from './App.svelte';

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
  vi.mocked(listen).mockResolvedValue(() => {});
  vi.mocked(listDevices).mockResolvedValue({ devices: [], current: null });
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
  const open = await screen.findAllByRole('button', { name: 'Open System Settings' });
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
  expect(screen.getByText('This cannot be undone.')).toBeTruthy();
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

it('holds the place words will take while the microphone is open', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes, open the pull request.')).toBeTruthy());

  daemon.update((s) => ({ ...s, live: { ...s.live, recording: true } }));
  await waitFor(() =>
    expect(screen.getByText('Recording. What you say will appear here.')).toBeTruthy(),
  );
  // NOT COVERED HERE: that the caret does not animate. jsdom answers `none` for
  // animationName whatever the stylesheet declares, so an assertion on it proves
  // nothing. The absence of keyframes outside Mark.svelte holds that line.
  const caret = container.querySelector('[data-mode="recording"] .caret');
  expect(caret).toBeTruthy();

  daemon.update((s) => ({ ...s, live: { ...s.live, recording: false, transcribing: true } }));
  await waitFor(() => expect(screen.getByText('Working out what you said.')).toBeTruthy());

  // Speaking produces no turn, so it must not hold a place for one.
  daemon.update((s) => ({ ...s, live: { ...s.live, transcribing: false, speaking: true } }));
  await waitFor(() => expect(container.querySelector('.caret')).toBeNull());
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

  const off = { ...ready, config: { ...ready.config, daemon: { save_history: false } } };
  daemon.update((s) => reduceStatus(s, off));
  // The daemon refuses the read outright while this is off.
  await waitFor(() => expect(screen.getByText('Nothing is kept')).toBeTruthy());
  expect(screen.queryByText('Yes, open the pull request.')).toBeNull();
});
