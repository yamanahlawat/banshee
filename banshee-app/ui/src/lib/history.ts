import type { HistoryRow } from './tauri';
import { sameLocalDay, toDate } from './time';

// A refresh reads the newest rows alone. Too few is not a wrong answer: a
// refresh that cannot find the row it last held reads the whole table again.
export const REFRESH_ROWS = 14;

// The daemon answers oldest first on both the limited and unlimited paths.
export function newestFirst(rows: HistoryRow[]): HistoryRow[] {
  return [...rows].reverse();
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
// in History instead. The rows arrive newest first, so the first row from
// another day ends the day and the whole table below it stays unread.
export function today(rows: HistoryRow[], now: Date, from = 0): HistoryRow[] {
  const kept: HistoryRow[] = [];
  for (let i = from; i < rows.length; i++) {
    if (!sameLocalDay(toDate(rows[i].timestamp), now)) break;
    kept.push(rows[i]);
  }
  return kept;
}

export function formatCount(n: number): string {
  return n.toLocaleString('en-US');
}
