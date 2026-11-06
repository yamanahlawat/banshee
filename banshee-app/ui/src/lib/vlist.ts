// Windowing math for the record: which rows mount, and how tall the two
// spacers stand so the scrollbar keeps the full length. Pure, so the suite
// holds it without laying anything out. Heights arrive per index once rows
// have mounted and been measured; unknown rows read as the estimate.
export type Window = { start: number; end: number; top: number; bottom: number };

export function plan(
  count: number,
  heightAt: (index: number) => number | null,
  estimate: number,
  scrollTop: number,
  viewport: number,
  overscan: number,
): Window {
  if (count <= 0) return { start: 0, end: 0, top: 0, bottom: 0 };
  const at = Math.max(0, scrollTop);
  const heights: number[] = [];
  let total = 0;
  for (let i = 0; i < count; i++) {
    heights.push(heightOf(heightAt, estimate, i));
    total += heights[i];
  }
  let start = 0;
  let run = 0;
  while (start < count && run + heights[start] <= at) {
    run += heights[start];
    start++;
  }
  let end = start;
  const edge = at + Math.max(0, viewport);
  while (end < count && run < edge) {
    run += heights[end];
    end++;
  }
  start = Math.max(0, start - overscan);
  end = Math.min(count, end + overscan);
  let top = 0;
  for (let i = 0; i < start; i++) top += heights[i];
  let shown = 0;
  for (let i = start; i < end; i++) shown += heights[i];
  return { start, end, top, bottom: Math.max(0, total - top - shown) };
}

function heightOf(heightAt: (index: number) => number | null, estimate: number, i: number): number {
  const h = heightAt(i) ?? estimate;
  return h > 0 ? h : estimate;
}

/// The height of the rows from `from` up to `to`, with the estimate standing in
/// for any not yet measured.
export function span(
  heightAt: (index: number) => number | null,
  estimate: number,
  from: number,
  to: number,
): number {
  let sum = 0;
  for (let i = from; i < to; i++) sum += heightOf(heightAt, estimate, i);
  return sum;
}

export type Segment = { spacer: number } | { rows: [start: number, end: number] };

// The window as the parts a list mounts. A kept index outside it, such as the
// focused row scrolled out of view, mounts at its true place between spacers.
export function segments(
  win: Window,
  kept: number | null,
  count: number,
  heightAt: (index: number) => number | null,
  estimate: number,
): Segment[] {
  const sum = (from: number, to: number) => span(heightAt, estimate, from, to);
  const parts: Segment[] = [];
  const spacer = (height: number) => {
    if (height > 0) parts.push({ spacer: height });
  };
  const mounted = (start: number, end: number): Segment => ({ rows: [start, end] });
  const outside =
    kept !== null && kept >= 0 && kept < count && (kept < win.start || kept >= win.end);
  if (!outside) {
    spacer(win.top);
    parts.push(mounted(win.start, win.end));
    spacer(win.bottom);
  } else if (kept < win.start) {
    spacer(sum(0, kept));
    parts.push(mounted(kept, kept + 1));
    spacer(sum(kept + 1, win.start));
    parts.push(mounted(win.start, win.end));
    spacer(win.bottom);
  } else {
    spacer(win.top);
    parts.push(mounted(win.start, win.end));
    spacer(sum(win.end, kept));
    parts.push(mounted(kept, kept + 1));
    spacer(sum(kept + 1, count));
  }
  return parts;
}
