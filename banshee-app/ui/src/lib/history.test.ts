import { beforeEach, describe, expect, it, vi } from 'vitest';

const { history } = vi.hoisted(() => ({ history: vi.fn() }));
vi.mock('./tauri', () => ({ history }));

import { get } from 'svelte/store';
import { countNewer, forget, formatCount, newestFirst, readNewest, table } from './history';

describe('newestFirst', () => {
  it('puts the daemon\'s last row first', () => {
    const rows = [
      { id: 1, text: 'first', timestamp: '2026-08-27T09:00:00Z' },
      { id: 2, text: 'second', timestamp: '2026-08-27T10:00:00Z' },
      { id: 3, text: 'third', timestamp: '2026-08-27T11:00:00Z' },
    ];
    expect(newestFirst(rows).map((r) => r.id)).toEqual([3, 2, 1]);
  });
});

describe('countNewer', () => {
  const page = [
    { id: 3, text: 'third', timestamp: '2026-08-27T11:00:00Z' },
    { id: 2, text: 'second', timestamp: '2026-08-27T10:00:00Z' },
    { id: 1, text: 'first', timestamp: '2026-08-27T09:00:00Z' },
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
      rows: [{ id: 2, text: 'second', timestamp: '2026-08-27T10:00:00Z' }],
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
    release([{ id: 2, text: 'second', timestamp: '2026-08-27T10:00:00Z' }]);
    await reading;

    expect(get(table).rows).toEqual([]);
    expect(get(table).total).toBe(0);
  });
});
