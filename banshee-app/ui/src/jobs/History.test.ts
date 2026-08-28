import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const { history, clearHistory, copyText, setSetting, status } = vi.hoisted(() => ({
  history: vi.fn(),
  clearHistory: vi.fn(),
  copyText: vi.fn(),
  setSetting: vi.fn(),
  status: vi.fn(),
}));
vi.mock('../lib/tauri', () => ({ history, clearHistory, copyText, setSetting, status }));
import ready from '../fixtures/ready.json';
import { daemon, empty, reduceStatus } from '../lib/daemon';
import { forgetCopy } from '../lib/copy';
import History from './History.svelte';

// Local noon, because `today()` reads the local calendar day: midday UTC is
// midnight in UTC+12 and every offset below would straddle it.
const NOW = new Date(2026, 7, 27, 12, 0, 0);
function stamp(minutesAgo: number): string {
  return new Date(NOW.getTime() - minutesAgo * 60_000).toISOString();
}

const ONE_ROW = [{ id: 1, text: 'Yes.', timestamp: stamp(30) }];

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  vi.clearAllMocks();
  forgetCopy();
  clearHistory.mockResolvedValue(null);
  copyText.mockResolvedValue(undefined);
  setSetting.mockResolvedValue([]);
  status.mockResolvedValue(ready);
  history.mockResolvedValue(ONE_ROW);
  daemon.set(reduceStatus(empty(), ready));
});

afterEach(() => vi.useRealTimers());

it('confirms in place with Cancel emphasised and clears only on the second press', async () => {
  render(History);
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear all' }));
  expect(screen.getByText(/This cannot be undone/)).toBeTruthy();
  expect(clearHistory).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: /^Clear 1$/ }));
  expect(clearHistory).toHaveBeenCalled();
});

it('dismisses the confirm without clearing when Cancel is pressed', async () => {
  render(History);
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear all' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  expect(screen.queryByText(/This cannot be undone/)).toBeNull();
  expect(clearHistory).not.toHaveBeenCalled();
});

it('expands a row on click and copies it with a control named for its time', async () => {
  render(History);
  const rowButton = await screen.findByRole('button', { name: /Show the whole dictation from/ });
  expect(rowButton.getAttribute('aria-expanded')).toBe('false');
  await fireEvent.click(rowButton);
  expect(screen.getByRole('button', { name: 'Collapse' }).getAttribute('aria-expanded')).toBe('true');

  const copyButton = screen.getByRole('button', { name: /Copy the dictation from/ });
  await fireEvent.click(copyButton);
  expect(copyText).toHaveBeenCalledWith('Yes.');
  expect(await screen.findByText('Copied')).toBeTruthy();
});

it('narrows the list to dictations matching the search', async () => {
  history.mockResolvedValue([
    { id: 1, text: 'Open the pull request.', timestamp: stamp(30) },
    { id: 2, text: 'Rename the reducer.', timestamp: stamp(60) },
  ]);
  render(History);
  await screen.findByText('Open the pull request.');
  const search = screen.getByRole('searchbox', { name: 'Search history' });
  await fireEvent.input(search, { target: { value: 'reducer' } });
  expect(screen.queryByText('Open the pull request.')).toBeNull();
  expect(screen.getByText('Rename the reducer.')).toBeTruthy();
});

it('hides a dictation from an earlier day until it is searched for', async () => {
  // The daemon answers in id order and writes each stamp as it inserts, so
  // the older row carries the lower id.
  history.mockResolvedValue([
    { id: 1, text: 'last week', timestamp: stamp(60 * 24 * 7) },
    { id: 2, text: 'Rename the reducer.', timestamp: stamp(30) },
  ]);
  render(History);
  await screen.findByText('Rename the reducer.');
  expect(screen.queryByText('last week')).toBeNull();
  expect(screen.getByText('Today · 1 older')).toBeTruthy();

  const search = screen.getByRole('searchbox', { name: 'Search history' });
  await fireEvent.input(search, { target: { value: 'last week' } });
  expect(screen.getByText('last week')).toBeTruthy();
  expect(screen.queryByText(/^Today ·/)).toBeNull();
  expect(screen.getByText('1 match')).toBeTruthy();
});

it('reads the settings the config already holds and writes changes through set_setting', async () => {
  render(History);
  const saveToggle = await screen.findByRole('switch', { name: 'Save history' });
  expect(saveToggle.getAttribute('aria-checked')).toBe('true');
  await fireEvent.click(saveToggle);
  expect(setSetting).toHaveBeenCalledWith('daemon.save_history', false);

  const alwaysToggle = screen.getByRole('switch', { name: 'Always on' });
  expect(alwaysToggle.getAttribute('aria-checked')).toBe('true');
  await fireEvent.click(alwaysToggle);
  expect(setSetting).toHaveBeenCalledWith('daemon.always_on', false);

  await fireEvent.click(screen.getByRole('radio', { name: 'Everything' }));
  expect(setSetting).toHaveBeenCalledWith('logging.level', 'debug');
});

it('says nothing was saved rather than showing an empty box with a pressable Clear all', async () => {
  history.mockResolvedValue([]);
  render(History);
  expect(await screen.findByText('Nothing saved yet')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Clear all' })).toBeNull();
});

it('marks a thousands count the way the artboard does', async () => {
  // Only one row needs to render today; the rest just pad the total so the
  // count crosses into four digits.
  const many = [
    { id: 1, text: 'Today one', timestamp: stamp(30) },
    ...Array.from({ length: 2313 }, (_, i) => ({ id: i + 2, text: `old ${i}`, timestamp: '2026-08-01T09:00:00Z' })),
  ];
  history.mockResolvedValue(many);
  render(History);
  expect(await screen.findByPlaceholderText('Search 2,314 dictations')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Clear all' }));
  expect(screen.getByText('Clear all 2,314 dictations? This cannot be undone.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Clear 2,314' })).toBeTruthy();
});
