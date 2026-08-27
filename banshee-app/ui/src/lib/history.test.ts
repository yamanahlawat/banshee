import { describe, expect, it } from 'vitest';
import { countNewer, moreCount, newestFirst, nextLimit, today } from './history';

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

describe('nextLimit', () => {
  it('is never zero when nothing has landed yet', () => {
    expect(nextLimit(0)).toBeGreaterThan(0);
  });
  it('grows as more rows are already shown', () => {
    expect(nextLimit(50)).toBeGreaterThan(nextLimit(0));
  });
});

describe('moreCount', () => {
  it('is the total less what is already shown', () => {
    expect(moreCount(2311, 3)).toBe(2308);
  });
  it('never goes negative when shown outgrows the last known total', () => {
    expect(moreCount(3, 5)).toBe(0);
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

describe('today', () => {
  const now = new Date('2026-08-27T12:00:00Z');
  const rows = [
    { id: 3, text: 'this morning', timestamp: '2026-08-27T06:00:00Z' },
    { id: 2, text: 'last night', timestamp: '2026-08-26T20:00:00Z' },
    { id: 1, text: 'last week', timestamp: '2026-08-20T09:00:00Z' },
  ];
  it('keeps only the rows from the reader\'s own day', () => {
    const kept = today(rows, now).map((r) => r.id);
    expect(kept).toContain(3);
    expect(kept).not.toContain(1);
  });
  it('keeps nothing when the newest row is from an earlier day', () => {
    expect(today(rows.slice(2), now)).toEqual([]);
  });
});
