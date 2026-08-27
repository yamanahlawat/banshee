import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ready from './fixtures/ready.json';

// Midday UTC is the same calendar day from -12:00 through +11:00, so the
// band's "today" rule reads the same under any host zone.
const NOW = new Date('2026-08-27T12:00:00Z');

function stamp(minutesAgo: number): string {
  return `${new Date(NOW.getTime() - minutesAgo * 60_000).toISOString().slice(0, 19)}Z`;
}

// The daemon answers oldest first, and a limit takes the newest rows.
const table = Array.from({ length: 10 }, (_, i) => ({
  id: i + 1,
  text: i === 9 ? 'Yes, open the pull request.' : `dictation ${i + 1}`,
  timestamp: stamp(9 - i),
}));

vi.mock('./lib/tauri', () => ({
  status: vi.fn(),
  history: vi.fn(),
  listen: vi.fn(),
  copyText: vi.fn(),
}));
import { history, listen, status } from './lib/tauri';
import { daemon, empty } from './lib/daemon';
import App from './App.svelte';

beforeEach(() => {
  // Only `Date` is faked: real timers keep the async waits below working.
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  vi.clearAllMocks();
  daemon.set(empty());
  vi.mocked(status).mockResolvedValue(ready);
  vi.mocked(history).mockImplementation(async (limit?: number) => (limit == null ? table : table.slice(-limit)));
  vi.mocked(listen).mockResolvedValue(() => {});
});

afterEach(() => vi.useRealTimers());

it('opens on Ready with the latest dictation and one live region', async () => {
  const { container } = render(App);
  expect(await screen.findByText('Yes, open the pull request.')).toBeTruthy();
  expect(container.querySelector('header')?.textContent).toContain('Ready');
  expect(container.querySelectorAll('[aria-live]').length).toBe(1);
});

it('keeps the live region off the bands, so a redraw is not announced', async () => {
  const { container } = render(App);
  await screen.findByText('Yes, open the pull request.');
  expect(container.querySelector('main')?.hasAttribute('aria-live')).toBe(false);
  expect(container.querySelector('.sr')?.getAttribute('aria-live')).toBe('polite');
});

it('renders a page of the table and counts the rest in the footer', async () => {
  render(App);
  // Ten rows: the pad takes the newest, the band takes three, six remain.
  expect(await screen.findByRole('button', { name: '6 more in History ›' })).toBeTruthy();
  expect(screen.queryByText('dictation 1')).toBeNull();
});

it('speaks a copy confirmation through the announcement region', async () => {
  const { container } = render(App);
  const copyButton = await screen.findByRole('button', { name: 'Copy' });
  await fireEvent.click(copyButton);
  // Pad renders its own visible "Copied", so this reads the region alone.
  expect(container.querySelector('.sr')?.textContent).toContain('Copied');
});

it('still listens when the daemon is not running at open', async () => {
  vi.mocked(status).mockRejectedValue(new Error('no socket'));
  vi.mocked(history).mockRejectedValue(new Error('no socket'));
  const { container } = render(App);
  await screen.findAllByText('Not running');
  expect(container.querySelector('header')?.textContent).toContain('Not running');
  const events = vi.mocked(listen).mock.calls.map((c) => c[0]);
  expect(events).toEqual(['daemon:status', 'daemon:state', 'daemon:downloads', 'daemon:down']);
});

it('reads the history once the daemon comes back', async () => {
  // Only the status read fails: the history read never runs while it does.
  vi.mocked(status).mockRejectedValueOnce(new Error('no socket'));
  render(App);
  await screen.findAllByText('Not running');
  const push = vi.mocked(listen).mock.calls.find((c) => c[0] === 'daemon:status')?.[1];
  push?.({ payload: ready } as never);
  expect(await screen.findByText('Yes, open the pull request.')).toBeTruthy();
});

it('empties the band when only the pad row is from today', async () => {
  const yesterday = [
    { id: 1, text: 'last week', timestamp: '2026-08-20T09:00:00Z' },
    { id: 2, text: 'Yes, open the pull request.', timestamp: stamp(1) },
  ];
  vi.mocked(history).mockImplementation(async () => yesterday);
  render(App);
  await screen.findByText('Yes, open the pull request.');
  expect(screen.queryByText('last week')).toBeNull();
  expect(await screen.findByRole('button', { name: '1 more in History \u203a' })).toBeTruthy();
});

function push(event: string, payload: unknown) {
  const handler = vi.mocked(listen).mock.calls.find((c) => c[0] === event)?.[1];
  handler?.({ payload } as never);
}

it('waits for transcribing to fall before it refetches, not for recording', async () => {
  render(App);
  await screen.findByText('Yes, open the pull request.');
  const reads = vi.mocked(history).mock.calls.length;

  // The daemon stops recording seconds before the row exists.
  push('daemon:state', { recording: true, transcribing: false });
  push('daemon:state', { recording: false, transcribing: false });
  expect(vi.mocked(history).mock.calls.length).toBe(reads);

  push('daemon:state', { recording: false, transcribing: true });
  push('daemon:state', { recording: false, transcribing: false });
  expect(vi.mocked(history).mock.calls.length).toBe(reads + 1);
});

it('knows a fall is coming when it opens mid-dictation', async () => {
  vi.mocked(status).mockResolvedValue({ ...ready, transcribing: true });
  render(App);
  await screen.findByText('Yes, open the pull request.');
  const reads = vi.mocked(history).mock.calls.length;

  push('daemon:state', { recording: false, transcribing: false });

  expect(vi.mocked(history).mock.calls.length).toBe(reads + 1);
});

it('reads the table once when the first read and a status push race', async () => {
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  vi.mocked(history).mockImplementation(async (limit?: number) => {
    if (limit == null) await gate;
    return limit == null ? table : table.slice(-limit);
  });

  render(App);
  // Let the listeners register and the first read start, then have the
  // bridge push its own status while that read is still in flight.
  for (let i = 0; i < 10; i++) await Promise.resolve();
  push('daemon:status', ready);
  release();

  await screen.findByText('Yes, open the pull request.');
  const wholeTableReads = vi.mocked(history).mock.calls.filter((c) => c[0] == null).length;
  expect(wholeTableReads).toBe(1);
});
