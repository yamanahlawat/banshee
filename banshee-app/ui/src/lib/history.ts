import type { HistoryRow } from './tauri';

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
