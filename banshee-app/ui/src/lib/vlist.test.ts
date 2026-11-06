import { describe, expect, it } from 'vitest';
import { plan, segments } from './vlist';

const at = (heights: (number | null)[]) => (i: number) => heights[i] ?? null;

describe('plan', () => {
  it('mounts nothing for an empty record', () => {
    expect(plan(0, at([]), 64, 0, 600, 3)).toEqual({ start: 0, end: 0, top: 0, bottom: 0 });
  });

  it('mounts the viewport plus overscan from the top', () => {
    const w = plan(10, at(Array(10).fill(60)), 64, 0, 200, 1);
    expect(w.start).toBe(0);
    expect(w.end).toBe(5);
    expect(w.top).toBe(0);
    expect(w.bottom).toBe((10 - 5) * 60);
  });

  it('slides the window with the scroll and pads both sides', () => {
    const w = plan(10, at(Array(10).fill(60)), 64, 300, 200, 1);
    expect(w.start).toBe(4);
    expect(w.end).toBe(10);
    expect(w.top).toBe(4 * 60);
    expect(w.bottom).toBe(0);
  });

  it('clamps the window and the spacers at the end', () => {
    const w = plan(10, at(Array(10).fill(60)), 64, 10000, 200, 2);
    expect(w.end).toBe(10);
    expect(w.bottom).toBe(0);
  });

  it('reads unknown rows as the estimate', () => {
    const w = plan(4, at([null, null, null, null]), 64, 0, 100, 0);
    expect(w.end).toBe(2);
    expect(w.bottom).toBe(2 * 64);
  });

  it('mixes measured rows with estimates without breaking the length', () => {
    const w = plan(3, at([100, null, 20]), 64, 0, 1000, 0);
    expect(w).toEqual({ start: 0, end: 3, top: 0, bottom: 0 });
  });

  it('treats a scrolled-past top as the top', () => {
    const w = plan(3, at([50, 50, 50]), 64, -40, 100, 1);
    expect(w.start).toBe(0);
    expect(w.top).toBe(0);
  });
});

describe('segments', () => {
  const heights = at(Array(10).fill(60));
  // Rows 4 to 6 mounted: 240 above, 180 below.
  const win = { start: 4, end: 7, top: 240, bottom: 180 };
  const of = (kept: number | null) => segments(win, kept, 10, heights, 64);

  it('is the window and its spacers when nothing is kept', () => {
    expect(of(null)).toEqual([{ spacer: 240 }, { rows: [4, 7] }, { spacer: 180 }]);
  });

  it('is the same when the kept row is inside the window', () => {
    expect(of(5)).toEqual(of(null));
  });

  it('holds a kept row above the window at its place', () => {
    expect(of(1)).toEqual([
      { spacer: 60 },
      { rows: [1, 2] },
      { spacer: 120 },
      { rows: [4, 7] },
      { spacer: 180 },
    ]);
  });

  it('holds a kept row below the window at its place', () => {
    expect(of(9)).toEqual([{ spacer: 240 }, { rows: [4, 7] }, { spacer: 120 }, { rows: [9, 10] }]);
  });

  it('drops spacers that would be empty', () => {
    expect(of(0)).toEqual([{ rows: [0, 1] }, { spacer: 180 }, { rows: [4, 7] }, { spacer: 180 }]);
  });

  it('reads an unmeasured row between as the estimate', () => {
    const w = segments(win, 1, 10, at([60, 60, null, 60, 60]), 64);
    expect(w[2]).toEqual({ spacer: 64 + 60 });
  });

  it('ignores a kept index the list no longer has', () => {
    expect(of(10)).toEqual(of(null));
    expect(of(-1)).toEqual(of(null));
  });
});
