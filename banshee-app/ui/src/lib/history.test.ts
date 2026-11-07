import { beforeEach, describe, expect, it, vi } from 'vitest';

const { history } = vi.hoisted(() => ({ history: vi.fn() }));
vi.mock('./tauri', () => ({ history }));

import { get } from 'svelte/store';
import { dictated } from './history.mock';
import {
  countNewer,
  forget,
  formatCount,
  newestFirst,
  readAll,
  readAfter,
  followSaveHistory,
  readNewest,
  table,
  dayLabel,
  marked,
  matches,
  rowFlag,
  selected,
  view,
  listed,
  clearSearch,
  copyOf,
  replaceRow,
  wouldDelete,
} from './history';
import type { HistoryRow } from './tauri';

describe('newestFirst', () => {
  it("puts the daemon's last row first", () => {
    const rows = [
      dictated(1, 'first', '2026-08-27T09:00:00Z'),
      dictated(2, 'second', '2026-08-27T10:00:00Z'),
      dictated(3, 'third', '2026-08-27T11:00:00Z'),
    ];
    expect(newestFirst(rows).map((r) => r.id)).toEqual([3, 2, 1]);
  });
});

describe('countNewer', () => {
  const page = [
    dictated(3, 'third', '2026-08-27T11:00:00Z'),
    dictated(2, 'second', '2026-08-27T10:00:00Z'),
    dictated(1, 'first', '2026-08-27T09:00:00Z'),
  ];
  it('counts the rows that landed above the newest one held', () => {
    expect(countNewer(page, 1)).toBe(2);
  });
  it('is zero when nothing landed', () => {
    expect(countNewer(page, 3)).toBe(0);
  });
  it('cannot answer when the page no longer holds that row', () => {
    expect(countNewer(page, 99)).toBeNull();
  });
  it('cannot answer before the first row is held', () => {
    expect(countNewer(page, null)).toBeNull();
  });
});

describe('formatCount', () => {
  it('reads a small count plainly', () => {
    expect(formatCount(7)).toBe('7');
  });
  it('marks the thousands the way the artboard does', () => {
    expect(formatCount(2314)).toBe('2,314');
  });
});

describe('readNewest', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    table.set({
      rows: [dictated(2, 'second', '2026-08-27T10:00:00Z')],
      total: 2,
      loaded: true,
      saving: true,
    });
  });

  it('does not put back rows a clear removed while it was reading', async () => {
    let release: (rows: unknown[]) => void = () => {};
    // Every read after the clear finds the daemon holding nothing.
    history
      .mockReturnValueOnce(new Promise((resolve) => (release = resolve)))
      .mockResolvedValue([]);

    const reading = readNewest();
    forget();
    release([dictated(2, 'second', '2026-08-27T10:00:00Z')]);
    await reading;

    expect(get(table).rows).toEqual([]);
    expect(get(table).total).toBe(0);
  });
});

describe('readAll', () => {
  // This path replaces the table wholesale, so a clear landing mid-read would
  // otherwise put every deleted row back, and the total with it.
  it('does not put back rows a clear removed while the whole table was reading', async () => {
    let release: (rows: unknown[]) => void = () => {};
    history.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));

    const reading = readAll();
    forget();
    release([dictated(1, 'gone', '2026-08-27T09:00:00Z')]);
    await reading;

    expect(get(table).rows).toEqual([]);
    expect(get(table).total).toBe(0);
  });
});

describe('readAfter', () => {
  beforeEach(() => {
    history.mockReset();
    forget();
  });

  it('reads again once a read already in flight lands', async () => {
    let land: (rows: HistoryRow[]) => void = () => {};
    history.mockReturnValueOnce(new Promise<HistoryRow[]>((resolve) => (land = resolve)));
    history.mockResolvedValueOnce([dictated(1, 'fresh', '2026-08-27T09:00:00Z')]);

    const old = readAll();
    const fresh = readAfter();
    land([dictated(1, 'stale', '2026-08-27T09:00:00Z')]);
    await Promise.all([old, fresh]);

    expect(history).toHaveBeenCalledTimes(2);
    expect(get(table).rows[0].text).toBe('fresh');
  });
});

describe('replaceRow', () => {
  it('replace_row_swaps_the_held_row_and_keeps_the_total', () => {
    const first = dictated(2, 'newer', '2026-08-27T09:05:00Z');
    const second = dictated(1, 'older', '2026-08-27T09:00:00Z');
    table.set({ rows: [first, second], total: 40, loaded: true, saving: true });
    replaceRow({ ...second, text: 'retried' });
    expect(get(table).rows).toEqual([first, { ...second, text: 'retried' }]);
    expect(get(table).total).toBe(40);
  });
});

describe('followSaveHistory', () => {
  it('leaves the rows kept when saving goes off', () => {
    const kept = [dictated(1, 'kept', '2026-08-27T09:00:00Z')];
    table.set({ rows: kept, total: 1, loaded: true, saving: true });
    followSaveHistory(false);
    expect(get(table).rows).toEqual(kept);
    expect(get(table).total).toBe(1);
    expect(get(table).saving).toBe(false);
  });
});

const row = (over: Partial<HistoryRow>): HistoryRow => ({
  id: 1,
  kind: 'question',
  question: null,
  text: null,
  outcome: 'transcribed',
  reason: null,
  timestamp: '2026-08-27T09:00:00Z',
  audio: null,
  ...over,
});

describe('matches', () => {
  it('finds the needle in a question, ignoring case', () => {
    expect(matches(row({ question: 'What is the Time', text: 'noon' }), 'time')).toBe(true);
  });
  it('finds the needle in an answer', () => {
    expect(matches(row({ question: 'what', text: 'It is Noon' }), 'noon')).toBe(true);
  });
  it('reads a null text as empty', () => {
    expect(matches(row({ question: 'what', text: null }), 'noon')).toBe(false);
    expect(matches(row({ question: null, text: null }), 'a')).toBe(false);
  });
  it('matches every row for an empty needle', () => {
    expect(matches(row({}), '')).toBe(true);
  });
});

describe('marked', () => {
  it('splits out every match, ignoring case and keeping the text as written', () => {
    expect(marked('Push it, then PUSH the tag', 'push')).toEqual([
      { text: 'Push', hit: true },
      { text: ' it, then ', hit: false },
      { text: 'PUSH', hit: true },
      { text: ' the tag', hit: false },
    ]);
  });
  it('reads the needle as plain text', () => {
    expect(marked('costs $5.00 (or 5x0)', '5.0')).toEqual([
      { text: 'costs $', hit: false },
      { text: '5.0', hit: true },
      { text: '0 (or 5x0)', hit: false },
    ]);
  });
  it('marks nothing for an empty needle', () => {
    expect(marked('noon', '')).toEqual([{ text: 'noon', hit: false }]);
  });
  // Case folding differs between toLowerCase and a regex: İ lowers to i plus a
  // combining dot, which a regex never folds to i.
  it.each([
    ['İstanbul', 'i'],
    ['costs $5.00', '5.0'],
    ['5x0', '5.0'],
    ['Push it', 'PUSH'],
  ])('marks %s for %s exactly when the row matches', (text, needle) => {
    const hit = marked(text, needle).some((part) => part.hit);
    expect(matches(row({ question: null, text }), needle)).toBe(hit);
  });
});

describe('rowFlag', () => {
  it('names a question nobody answered', () => {
    expect(rowFlag(row({ outcome: 'no_answer' }))).toBe('No answer heard');
  });
  it('names a question closed unanswered', () => {
    expect(rowFlag(row({ outcome: 'closed' }))).toBe('Closed before you answered');
  });
  it('names a question whose answer failed', () => {
    expect(rowFlag(row({ outcome: 'failed', reason: 'model down' }))).toBe('Not transcribed');
  });
  it('names a question never asked aloud', () => {
    expect(rowFlag(row({ outcome: 'unasked' }))).toBe('Not asked aloud');
  });
  it('gives a failed dictation its reason', () => {
    expect(rowFlag(row({ kind: 'dictation', outcome: 'failed', reason: 'model down' }))).toBe(
      'model down',
    );
  });
  it('flags nothing for a transcribed row', () => {
    expect(rowFlag(row({}))).toBeNull();
    expect(rowFlag(row({ kind: 'dictation' }))).toBeNull();
  });
});

describe('dayLabel', () => {
  const now = new Date(2026, 10, 6, 15, 0);
  const at = (y: number, m: number, d: number) => new Date(y, m, d, 23, 58).toISOString();
  it('names today and yesterday', () => {
    expect(dayLabel(at(2026, 10, 6), now)).toBe('Today');
    expect(dayLabel(at(2026, 10, 5), now)).toBe('Yesterday');
  });
  it('spells the month for an older day this year', () => {
    expect(dayLabel(at(2026, 7, 27), now)).toBe('27 August');
  });
  it('adds the year for an earlier year', () => {
    expect(dayLabel(at(2025, 11, 31), now)).toBe('31 December 2025');
  });
  it('counts back by the calendar, so a clock change cannot skip a day', () => {
    const firstOfMarch = new Date(2026, 2, 1, 0, 30, 0);
    expect(dayLabel(new Date(2026, 1, 28, 22, 0).toISOString(), firstOfMarch)).toBe('Yesterday');
  });
});

describe('selected', () => {
  const shown = [row({ id: 3 }), row({ id: 2 }), row({ id: 1 })];
  it('is the chosen row while it is shown', () => {
    expect(selected(shown, 2)?.id).toBe(2);
  });
  it('falls to the first shown row when the choice is gone', () => {
    expect(selected(shown, 9)?.id).toBe(3);
    expect(selected(shown, null)?.id).toBe(3);
  });
  it('is null when nothing is shown', () => {
    expect(selected([], 2)).toBeNull();
  });
});

describe('view', () => {
  it('is cleared by forget', () => {
    view.set({ query: 'time', chosen: 4 });
    forget();
    expect(get(view)).toEqual({ query: '', chosen: null });
  });
});

describe('listed', () => {
  const rows = [row({ id: 3, text: 'Open it' }), row({ id: 2, text: 'close it' }), row({ id: 1 })];
  beforeEach(() => {
    table.set({ rows, total: 3, loaded: true, saving: true });
    view.set({ query: '', chosen: null });
  });
  it('is every row while the search is empty or blank', () => {
    expect(get(listed)).toEqual(rows);
    view.set({ query: '   ', chosen: null });
    expect(get(listed)).toEqual(rows);
  });
  it('is the rows that match the trimmed search', () => {
    view.set({ query: ' CLOSE ', chosen: null });
    expect(get(listed).map((one) => one.id)).toEqual([2]);
  });
});

describe('clearSearch', () => {
  const rows = [row({ id: 3, text: 'newest' }), row({ id: 2, text: 'middle' }), row({ id: 1 })];
  beforeEach(() => table.set({ rows, total: 3, loaded: true, saving: true }));

  it('keeps the row the search had selected', () => {
    view.set({ query: 'middle', chosen: null });
    clearSearch();
    expect(get(view)).toEqual({ query: '', chosen: 2 });
  });
  it('keeps the first match when the chosen row was hidden', () => {
    view.set({ query: 'middle', chosen: 1 });
    clearSearch();
    expect(get(view)).toEqual({ query: '', chosen: 2 });
  });
  it('keeps following the newest when the newest was selected and nothing was chosen', () => {
    view.set({ query: 'newest', chosen: null });
    clearSearch();
    expect(get(view)).toEqual({ query: '', chosen: null });
  });
  it('keeps the choice when nothing matched', () => {
    view.set({ query: 'nothing', chosen: 1 });
    clearSearch();
    expect(get(view)).toEqual({ query: '', chosen: 1 });
  });
});

describe('copyOf', () => {
  it('copies a dictation as its text', () => {
    expect(copyOf(row({ kind: 'dictation', text: 'said' }))).toEqual({
      text: 'said',
      which: 'text',
    });
  });
  it('copies an answered question as its answer', () => {
    expect(copyOf(row({ question: 'asked', text: 'answered' }))).toEqual({
      text: 'answered',
      which: 'answer',
    });
  });
  it('copies an unanswered question as its question', () => {
    expect(copyOf(row({ question: 'asked', outcome: 'no_answer' }))).toEqual({
      text: 'asked',
      which: 'question',
    });
  });
  it('copies nothing from a failed dictation', () => {
    expect(copyOf(row({ kind: 'dictation', outcome: 'failed', reason: 'x' }))).toBeNull();
  });
});

describe('wouldDelete', () => {
  const now = new Date('2026-11-07T12:00:00Z');
  const kept = (id: number, timestamp: string, until: string | null): HistoryRow => ({
    ...dictated(id, 'said', timestamp),
    audio: { seconds: 2, until },
  });
  const rows = [
    kept(1, '2026-11-07T10:00:00Z', '2026-11-14T10:00:00Z'),
    kept(2, '2026-11-06T12:00:00Z', '2026-11-13T12:00:00Z'),
    kept(3, '2026-11-05T12:00:00Z', '2026-11-12T12:00:00Z'),
    kept(4, '2026-10-20T12:00:00Z', null),
    dictated(5, 'no audio', '2026-09-01T12:00:00Z'),
    // Past its own end, so the daemon has already let it go.
    kept(6, '2026-10-01T12:00:00Z', '2026-10-08T12:00:00Z'),
  ];

  it('counts the rows whose audio the new period puts past its end', () => {
    expect(wouldDelete(rows, 'off', now)).toBe(4);
    expect(wouldDelete(rows, '1d', now)).toBe(3);
    expect(wouldDelete(rows, '7d', now)).toBe(1);
    expect(wouldDelete(rows, '30d', now)).toBe(0);
    expect(wouldDelete(rows, 'forever', now)).toBe(0);
  });

  // Each reads as a number of days to a parser, so a guess would count rows.
  it.each(['2d', '1w'])('counts nothing for a period it does not know, %j', (period) => {
    expect(wouldDelete(rows, period, now)).toBe(0);
  });
});
