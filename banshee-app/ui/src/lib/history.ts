import { derived, get, writable, type Readable, type Writable } from 'svelte/store';
import { history, type HistoryRow } from './tauri';
import { sameLocalDay, toDate } from './time';

// A refresh that cannot find the row it last held reads the whole table again,
// so too few here is not a wrong answer.
const REFRESH_ROWS = 14;

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

export type Table = {
  rows: HistoryRow[];
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

// Moved by every clear, so a read that began before one can tell its answer
// describes a table that no longer exists.
let cleared = 0;

export function readAll(): Promise<void> {
  if (reading === null) {
    // Only if it is still the current read: `forget` may have dropped it and a
    // later caller claimed the slot, and clearing that would send a third
    // caller down its own round trip.
    const mine: Promise<void> = readWholeTable().finally(() => {
      if (reading === mine) reading = null;
    });
    reading = mine;
  }
  return reading;
}

/// A whole-table read that begins after this call, for a caller that has just
/// changed what the daemon answers: a read already in flight may hold the old
/// answer.
export async function readAfter(): Promise<void> {
  if (reading !== null) await reading.catch(() => {});
  return readAll();
}

async function readWholeTable(): Promise<void> {
  const was = cleared;
  const all = await history();
  if (cleared !== was) return;
  table.update((held) => ({ ...held, rows: newestFirst(all), total: all.length, loaded: true }));
}

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
// its side.
export function forget(): void {
  cleared += 1;
  // The read in flight will apply nothing, so a caller that asks after this
  // must get its own rather than be handed the one already emptied.
  reading = null;
  view.set({ query: '', chosen: null });
  table.update((held) => ({ ...held, rows: [], total: 0, loaded: true }));
}

/// Puts a row the daemon changed in place of the one held with its id. The
/// table keeps as many rows as before, so the total stands.
export function replaceRow(row: HistoryRow): void {
  table.update((held) => ({
    ...held,
    rows: held.rows.map((one) => (one.id === row.id ? row : one)),
  }));
}

/// The rows whose retry is in flight. Held here, not in the detail, so a
/// detail mounted again mid-retry still reads it.
export const retrying: Writable<ReadonlySet<HistoryRow['id']>> = writable(new Set());

// Follows what the daemon says it is keeping. The open reads the table itself,
// so the first answer only records the switch. Rows kept earlier stay listed
// with it off.
export function followSaveHistory(on: boolean): void {
  const held = get(table);
  if (on === held.saving) return;
  const first = held.saving === null;
  table.update((now) => ({ ...now, saving: on }));
  if (on && !first) readAll().catch(() => {});
}

/// The read a caller wants: the whole table the first time, the newest rows
/// after that. Which one is the table's own business, not each caller's.
export function readLatest(): Promise<void> {
  return (get(table).loaded ? readNewest() : readAll()).catch(() => {});
}

export type View = { query: string; chosen: HistoryRow['id'] | null };

export const view: Writable<View> = writable({ query: '', chosen: null });

// One pattern finds and marks, so a listed row always shows why it matched.
// Kept for the last needle: every row asks with the same one.
let held: { needle: string; pattern: RegExp } | null = null;
function patternOf(needle: string): RegExp {
  if (held?.needle !== needle) {
    const literal = needle.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    held = { needle, pattern: new RegExp(`(${literal})`, 'iu') };
  }
  return held.pattern;
}

export function matches(row: HistoryRow, needle: string): boolean {
  const pattern = patternOf(needle);
  return pattern.test(row.question ?? '') || pattern.test(row.text ?? '');
}

/// The text in runs, each a match for the needle or not, for drawing marks.
export function marked(text: string, needle: string): { text: string; hit: boolean }[] {
  if (needle === '') return [{ text, hit: false }];
  return text
    .split(patternOf(needle))
    .map((part, i) => ({ text: part, hit: i % 2 === 1 }))
    .filter((part) => part.text !== '');
}

/// Rows are newest first, so the first match is the newest.
export function newestUnanswered(rows: HistoryRow[]): HistoryRow | null {
  return rows.find((row) => row.kind === 'question' && row.outcome === 'no_answer') ?? null;
}

/// Who asked a question. The meta line reads it in lower case where no client
/// is stored.
export function asker(row: HistoryRow, lower = false): string {
  if (row.client) return row.client;
  return lower ? 'an agent' : 'An agent';
}

export function rowFlag(row: HistoryRow): string | null {
  if (row.kind === 'dictation') return row.outcome === 'failed' ? row.reason : null;
  switch (row.outcome) {
    case 'no_answer':
      return 'No answer heard';
    case 'closed':
      return 'Closed before you answered';
    case 'failed':
      return 'Not transcribed';
    case 'unasked':
      return 'Not asked aloud';
    default:
      return null;
  }
}

const MONTHS = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];

// Counted back by the calendar, so a clock change cannot skip a day.
export function dayLabel(timestamp: string, now: Date): string {
  const at = toDate(timestamp);
  if (sameLocalDay(at, now)) return 'Today';
  if (sameLocalDay(at, new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1))) {
    return 'Yesterday';
  }
  const day = dayAndMonth(at);
  return at.getFullYear() === now.getFullYear() ? day : `${day} ${at.getFullYear()}`;
}

function dayAndMonth(at: Date): string {
  return `${at.getDate()} ${MONTHS[at.getMonth()]}`;
}

/// The row's audio while it is still kept at `now`.
export function keptAudio(row: HistoryRow, now: Date): HistoryRow['audio'] {
  const audio = row.audio;
  if (audio === null || (audio.until !== null && toDate(audio.until) <= now)) return null;
  return audio;
}

const DAY_MS = 86_400_000;

const KEEP_DAYS: Record<string, number> = {
  off: 0,
  '1d': 1,
  '7d': 7,
  '30d': 30,
  forever: Infinity,
};

/// How many rows keep audio that `period` would put past its end at `now`.
export function wouldDelete(rows: HistoryRow[], period: string, now: Date): number {
  const days = KEEP_DAYS[period] ?? Infinity;
  const limit = now.getTime() - days * DAY_MS;
  return rows.filter(
    (row) => keptAudio(row, now) !== null && toDate(row.timestamp).getTime() <= limit,
  ).length;
}

export function keptUntil(until: string | null): string {
  return until === null
    ? 'Audio kept until you clear it'
    : `Audio kept until ${dayAndMonth(toDate(until))}`;
}

export function selected(shown: HistoryRow[], chosen: HistoryRow['id'] | null): HistoryRow | null {
  return shown.find((row) => row.id === chosen) ?? shown[0] ?? null;
}

// A store of its own, so a new selection does not filter the table again.
export const needle: Readable<string> = derived(view, (now) => now.query.trim());

/// The rows the search leaves, newest first.
export const listed: Readable<HistoryRow[]> = derived([table, needle], ([held, query]) =>
  query === '' ? held.rows : held.rows.filter((row) => matches(row, query)),
);

/// Empties the search and keeps the row it had selected. Selection goes on
/// following the newest row only if it was doing so and still lands there.
export function clearSearch(): void {
  const newest = get(table).rows[0]?.id ?? null;
  const held = selected(get(listed), get(view).chosen);
  view.update(({ chosen }) => ({
    query: '',
    chosen: held === null || (chosen === null && held.id === newest) ? chosen : held.id,
  }));
}

/// What Cmd+C takes from a row: the answer or the text, else the question.
/// `which` names the detail's copy button that shows it was copied.
export function copyOf(
  row: HistoryRow,
): { text: string; which: 'text' | 'answer' | 'question' } | null {
  if (row.text) return { text: row.text, which: row.kind === 'dictation' ? 'text' : 'answer' };
  if (row.question) return { text: row.question, which: 'question' };
  return null;
}
