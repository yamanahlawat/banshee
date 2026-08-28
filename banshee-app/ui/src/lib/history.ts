import type { HistoryRow } from './tauri';
import { sameLocalDay, toDate } from './time';

const BAND_ROWS = 3;

// The pad holds the newest row above the band.
export const PAGE = BAND_ROWS + 1;

// The daemon answers oldest first on both the limited and unlimited paths.
export function newestFirst(rows: HistoryRow[]): HistoryRow[] {
  return [...rows].reverse();
}

// A limit of zero returns no rows at all, so the margin keeps this above
// zero even before the first row of the session has landed.
export function nextLimit(shown: number, margin = 10): number {
  return shown + margin;
}

export function moreCount(total: number, shown: number): number {
  return Math.max(total - shown, 0);
}

// The daemon has no count call, so the total moves by the rows above the newest
// one already held. A page that no longer holds that row answers null.
export function countNewer(page: HistoryRow[], newestId: HistoryRow['id'] | null): number | null {
  if (newestId === null) return null;
  const at = page.findIndex((row) => row.id === newestId);
  return at === -1 ? null : at;
}

// The band is headed "Earlier today", so a row from any earlier day belongs
// in History instead.
export function today(rows: HistoryRow[], now: Date): HistoryRow[] {
  return rows.filter((row) => sameLocalDay(toDate(row.timestamp), now));
}

// A daemon that has run for months holds thousands of dictations, and a raw
// digit string reads as a wall of numbers without this.
export function formatCount(n: number): string {
  return n.toLocaleString('en-US');
}
