import { describe, expect, it } from 'vitest';
import { moreCount, newestFirst, nextLimit } from './history';

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
