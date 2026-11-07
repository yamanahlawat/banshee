import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { history, setSetting, status } from '../lib/tauri';
import type { HistoryRow } from '../lib/tauri';
import { daemon, type Status } from '../lib/daemon';
import { table } from '../lib/settings';
import { table as historyTable } from '../lib/history';
import { dictated } from '../lib/history.mock';
import { commonTable } from '../lib/tauri.mock';
import SavedHistoryPage from './SavedHistoryPage.svelte';

const NOW = new Date('2026-11-07T12:00:00Z');

const kept = (id: number, timestamp: string): HistoryRow => ({
  ...dictated(id, 'said', timestamp),
  audio: { seconds: 2, until: null },
});

// Two days old and ten days old, so 7 days deletes one and 30 days deletes none.
const RECENT = kept(1, '2026-11-05T12:00:00Z');
const OLDER = kept(2, '2026-10-28T12:00:00Z');

function ready(keepAudio?: string) {
  const config: Record<string, unknown> = {};
  if (keepAudio !== undefined) config.daemon = { keep_audio: keepAudio };
  const status = { running: true, config } as unknown as Status;
  daemon.update((state) => ({ ...state, status, pending: new Set<string>() }));
}

function holding(rows: HistoryRow[]) {
  historyTable.set({ rows, total: rows.length, loaded: true, saving: true });
}

function renderPage(keepAudio: string | undefined, rows: HistoryRow[]) {
  ready(keepAudio);
  holding(rows);
  return render(SavedHistoryPage, { saving: true });
}

const period = (name: string) => screen.getByRole('radio', { name });

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  table.set(commonTable);
  vi.mocked(setSetting).mockReset();
  vi.mocked(setSetting).mockResolvedValue([] as never);
  vi.mocked(history).mockReset();
  vi.mocked(history).mockResolvedValue([RECENT, OLDER]);
  vi.mocked(status).mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

it('shows the Keep audio period in force, and the default when the daemon names none', () => {
  const page = renderPage('30d', []);
  expect(period('30 days').getAttribute('aria-checked')).toBe('true');
  const group = screen.getByRole('radiogroup', { name: 'Keep audio' });
  const note = screen.getByText(
    'Audio holds whatever the microphone heard, other voices too. A shorter period deletes older audio.',
  );
  expect(group.getAttribute('aria-describedby')).toBe(note.id);
  page.unmount();

  renderPage(undefined, []);
  expect(period('7 days').getAttribute('aria-checked')).toBe('true');
});

it('writes a longer period at once', async () => {
  renderPage('7d', [RECENT, OLDER]);
  await fireEvent.click(period('30 days'));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('daemon.keep_audio', '30d'));
  expect(screen.queryByText(/This cannot be undone/)).toBeNull();
});

it('writes a shorter period at once when it would delete nothing', async () => {
  renderPage('30d', [RECENT]);
  await fireEvent.click(period('7 days'));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('daemon.keep_audio', '7d'));
  expect(screen.queryByText(/This cannot be undone/)).toBeNull();
});

it('asks first when a shorter period would delete kept audio', async () => {
  renderPage('30d', [RECENT, OLDER]);
  await fireEvent.click(period('7 days'));

  expect(
    screen.getByText('Delete audio older than 7 days? 1 recording. This cannot be undone.'),
  ).toBeTruthy();
  expect(screen.queryByRole('radiogroup', { name: 'Keep audio' })).toBeNull();
  const keep = screen.getByRole('button', { name: 'Keep it' });
  await waitFor(() => expect(document.activeElement).toBe(keep));
  expect(setSetting).not.toHaveBeenCalled();
});

it('names all kept audio when Off would delete it', async () => {
  renderPage('7d', [RECENT, OLDER]);
  await fireEvent.click(period('Off'));
  expect(
    screen.getByText('Delete all kept audio? 2 recordings. This cannot be undone.'),
  ).toBeTruthy();
});

// An arrow press commits, so the question comes from the keyboard as often as
// from a click. The control comes back with its stop on the period in force.
it('writes nothing on Keep it, and puts the control back on the period in force', async () => {
  renderPage('30d', [RECENT, OLDER]);
  const was = period('30 days');
  was.focus();
  await fireEvent.keyDown(was, { key: 'ArrowLeft' });

  await fireEvent.click(await screen.findByRole('button', { name: 'Keep it' }));

  expect(setSetting).not.toHaveBeenCalled();
  const back = period('30 days');
  expect(back.getAttribute('aria-checked')).toBe('true');
  expect(back.tabIndex).toBe(0);
  await waitFor(() => expect(document.activeElement).toBe(back));
});

// The daemon's answer is a round trip away, so until it lands the control
// shows the period just chosen, where focus already is.
it('writes the shorter period on Delete, and reads the table again', async () => {
  let land: () => void = () => {};
  vi.mocked(setSetting).mockReturnValue(new Promise((resolve) => (land = () => resolve([]))));
  vi.mocked(status).mockResolvedValue({
    running: true,
    config: { daemon: { keep_audio: '7d' } },
  } as unknown as Status);
  renderPage('30d', [RECENT, OLDER]);
  await fireEvent.click(period('7 days'));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

  await waitFor(() => expect(document.activeElement).toBe(period('7 days')));
  expect(period('7 days').tabIndex).toBe(0);
  expect(period('7 days').getAttribute('aria-checked')).toBe('true');
  expect(screen.queryByText(/This cannot be undone/)).toBeNull();

  land();
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('daemon.keep_audio', '7d'));
  await waitFor(() => expect(history).toHaveBeenCalled());
  expect(period('7 days').getAttribute('aria-checked')).toBe('true');
});

it('lands on the period in force when the write is refused', async () => {
  vi.mocked(setSetting).mockRejectedValue(new Error('refused'));
  renderPage('30d', [RECENT, OLDER]);
  await fireEvent.click(period('7 days'));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

  await waitFor(() => expect(document.activeElement).toBe(period('30 days')));
  expect(period('30 days').getAttribute('aria-checked')).toBe('true');
  expect(period('30 days').tabIndex).toBe(0);
});

it('neither writes nor asks for the period already in force', async () => {
  renderPage('7d', [RECENT, OLDER]);
  await fireEvent.click(period('7 days'));
  expect(screen.queryByText(/This cannot be undone/)).toBeNull();
  expect(setSetting).not.toHaveBeenCalled();
});

// Before the first read the table holds no rows, and a count of none would
// write a shorter period without asking.
it('reads the table before counting when it has not been read yet', async () => {
  ready('30d');
  historyTable.set({ rows: [], total: 0, loaded: false, saving: true });
  render(SavedHistoryPage, { saving: true });
  await fireEvent.click(period('7 days'));

  expect(
    await screen.findByText('Delete audio older than 7 days? 1 recording. This cannot be undone.'),
  ).toBeTruthy();
  expect(setSetting).not.toHaveBeenCalled();
});

// VoiceOver reads a button's name and description, never the paragraph
// above it.
it('points both answers of each question at the question', async () => {
  renderPage('30d', [RECENT, OLDER]);
  await fireEvent.click(period('7 days'));
  const audio = screen.getByText(/^Delete audio older than 7 days\?/);
  for (const name of ['Keep it', 'Delete']) {
    expect(screen.getByRole('button', { name }).getAttribute('aria-describedby')).toBe(audio.id);
  }
  expect(audio.id).not.toBe('');
  await fireEvent.click(screen.getByRole('button', { name: 'Keep it' }));

  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  const clear = screen.getByText(/^Delete all 2 rows/);
  for (const name of ['Keep it', 'Delete everything']) {
    expect(screen.getByRole('button', { name }).getAttribute('aria-describedby')).toBe(clear.id);
  }
  expect(clear.id).not.toBe('');
});

it('names the audio in the Clear question when any row has it', async () => {
  const page = renderPage('7d', [RECENT, dictated(3, 'said', '2026-11-06T12:00:00Z')]);
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  expect(
    screen.getByText('Delete all 2 rows and their audio? This cannot be undone.'),
  ).toBeTruthy();
  page.unmount();

  renderPage('7d', [
    dictated(3, 'said', '2026-11-06T12:00:00Z'),
    dictated(4, 'said', '2026-11-05T12:00:00Z'),
  ]);
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  expect(screen.getByText('Delete all 2 rows? This cannot be undone.')).toBeTruthy();
});
