import { describe, expect, it } from 'vitest';
import { countNewer, formatCount, moreCount, newestFirst, today } from './history';

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
  // `today` reads the local calendar day, so noon local anchors the fixture
  // and every offset below stays on the day its name claims in any zone.
  const now = new Date(2026, 7, 27, 12, 0, 0);
  const hoursBefore = (h: number) => new Date(now.getTime() - h * 3_600_000).toISOString();
  const rows = [
    { id: 3, text: 'an hour ago', timestamp: hoursBefore(1) },
    { id: 2, text: 'this morning', timestamp: hoursBefore(4) },
    { id: 1, text: 'last week', timestamp: hoursBefore(24 * 7) },
  ];
  it('keeps only the rows from the reader\'s own day', () => {
    expect(today(rows, now).map((r) => r.id)).toEqual([3, 2]);
  });
  it('keeps nothing when the newest row is from an earlier day', () => {
    expect(today(rows.slice(2), now)).toEqual([]);
  });
  it('starts where it is told, so the pad\'s own row is not counted twice', () => {
    expect(today(rows, now, 1).map((r) => r.id)).toEqual([2]);
  });
  it('reads no further than the first row from another day', () => {
    let reads = 0;
    const counted = rows.map((row) => ({ ...row, get timestamp() { reads++; return row.timestamp; } }));
    today(counted, now);
    expect(reads).toBe(3);
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
