import { get, writable, type Writable } from 'svelte/store';
import { history, type HistoryRow } from './tauri';

// A refresh that cannot find the row it last held reads the whole table again,
// so too few here is not a wrong answer.
export const REFRESH_ROWS = 14;

// The daemon answers oldest first on both the limited and unlimited paths.
export function newestFirst(rows: HistoryRow[]): HistoryRow[] {
  return [...rows].reverse();
}

// The daemon has no count call, so the total moves by the rows above the one
// already held. A page that no longer holds it answers null.
export function countNewer(page: HistoryRow[], newestId: HistoryRow['id'] | null): number | null {
  if (newestId === null) return null;
  const at = page.findIndex((row) => row.id === newestId);
  return at === -1 ? null : at;
}

export function formatCount(n: number): string {
  return n.toLocaleString('en-US');
}

// One table for the pad, the earlier band and the History panel.
export type Table = {
  rows: HistoryRow[];
  // Whether the daemon last said it was keeping anything.
  saving: boolean | null;
  // The one unlimited read's answer, moved by hand afterwards.
  total: number;
  // False until a read lands, so an empty table and an unread one differ.
  loaded: boolean;
};

export const table: Writable<Table> = writable({ rows: [], total: 0, loaded: false, saving: null });

// The first read and the bridge's status push both arrive before either
// finishes, so they share one read of the table.
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
  // After the await, not before: a `Clear all` landing mid-flight would
  // otherwise be undone by an older snapshot.
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

// Empties the table without asking the daemon: `Clear all` has already emptied
// its side, and history switched off keeps nothing to read.
export function forget(): void {
  table.update((held) => ({ ...held, rows: [], total: 0, loaded: true }));
}

// Follows what the daemon says it is keeping. The open reads the table itself,
// so the first answer only records the switch. Off is the exception: nothing is
// there to read.
export function followSaveHistory(on: boolean): void {
  const held = get(table);
  if (on === held.saving) return;
  const first = held.saving === null;
  table.update((now) => ({ ...now, saving: on }));
  if (!on) forget();
  else if (!first) readAll().catch(() => {});
}

/// The read a caller wants: the whole table the first time, the newest rows
/// after that. Which one is the table's own business, not each caller's.
export function readLatest(): Promise<void> {
  return (get(table).loaded ? readNewest() : readAll()).catch(() => {});
}
