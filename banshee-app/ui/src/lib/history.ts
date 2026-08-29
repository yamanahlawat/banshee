import { get, writable, type Writable } from 'svelte/store';
import { history, type HistoryRow } from './tauri';
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

/// What the pad, the earlier band and the History panel all read. One table,
/// so a dictation, a clear and a `save_history` change move the three of them
/// together rather than each one waiting to be opened again.
export type Table = {
  rows: HistoryRow[];
  // What `daemon.save_history` last said. Held here rather than in a caller so
  // that one reset returns the table and the switch together.
  saving: boolean | null;
  // The daemon has no count call, so this is the one unlimited read's answer,
  // moved by hand afterwards.
  total: number;
  // False until a read lands, so an empty table and an unread one differ.
  loaded: boolean;
};

export const table: Writable<Table> = writable({ rows: [], total: 0, loaded: false, saving: null });

// The first read and the bridge's own status push both reach this while
// neither has finished, so they share one read of the table.
let reading: Promise<void> | null = null;

export function readAll(): Promise<void> {
  reading ??= readWholeTable().finally(() => {
    reading = null;
  });
  return reading;
}

async function readWholeTable(): Promise<void> {
  const all = await history();
  table.update((held) => ({ ...held, rows: newestFirst(all), total: all.length, loaded: true }));
}

// The daemon stores the row before it reports transcribing finished, so that
// fall is the first moment a refetch can see the new dictation.
export async function readNewest(): Promise<void> {
  const page = newestFirst(await history(REFRESH_ROWS));
  // Read after the await, not before: a `Clear all` or a `save_history` write
  // that lands while this is in flight would otherwise be undone by a snapshot
  // taken before either of them.
  const held = get(table);
  const added = countNewer(page, held.rows[0]?.id ?? null);
  if (added === null) {
    await readAll();
    return;
  }
  table.update((now) => ({
    ...now,
    rows: [...page.slice(0, added), ...held.rows],
    total: held.total + added,
    loaded: true,
  }));
}

/// Empties the table without asking the daemon. `Clear all` has already
/// emptied its side, and history switched off keeps nothing to read.
export function forget(): void {
  table.update((held) => ({ ...held, rows: [], total: 0, loaded: true }));
}

/// Follows `daemon.save_history`. The open reads the table itself, so the first
/// answer only records the switch. Off is the exception: nothing is there to
/// read, and the panel must say so rather than wait.
export function followSaveHistory(on: boolean): void {
  const held = get(table);
  if (on === held.saving) return;
  const first = held.saving === null;
  table.update((now) => ({ ...now, saving: on }));
  if (!on) forget();
  else if (!first) readAll().catch(() => {});
}

