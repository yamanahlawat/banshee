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
    const h = heightAt(i) ?? estimate;
    heights.push(h > 0 ? h : estimate);
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
