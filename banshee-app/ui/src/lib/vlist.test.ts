import { describe, expect, it } from 'vitest';
import { plan } from './vlist';

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
    expect(w.top + [w.start, w.end].length).toBeGreaterThan(0);
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
