// Every row the daemon answers names its zone.
export function toDate(stamp: string): Date {
  return new Date(stamp);
}

export function formatTime(stamp: string): string {
  const d = toDate(stamp);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

// A clock time alone reads as today. One minute after midnight the newest
// dictation is yesterday's, and it must not say 23:58 and nothing more.
const MONTHS_IN_FULL = [
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
const MONTHS = MONTHS_IN_FULL.map((month) => month.slice(0, 3));

// The day before the one the reader is in, counted by the calendar so a clock
// change cannot skip it.
function isYesterday(at: Date, now: Date): boolean {
  return sameLocalDay(at, new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1));
}

export function formatWhen(stamp: string, now: Date): string {
  const at = toDate(stamp);
  if (sameLocalDay(at, now)) return formatTime(stamp);
  if (isYesterday(at, now)) return `Yesterday ${formatTime(stamp)}`;
  // The year takes the clock's place rather than joining it: a dictation that
  // old is looked up by when, not by what minute.
  if (at.getFullYear() !== now.getFullYear()) {
    return `${at.getDate()} ${MONTHS[at.getMonth()]} ${at.getFullYear()}`;
  }
  return `${at.getDate()} ${MONTHS[at.getMonth()]} ${formatTime(stamp)}`;
}

// The day a record starts, named the way a sentence names it rather than the
// way a gutter does. The month is spelled out: this reads in prose, not in the
// mono column, and an abbreviation there would be the machine voice.
export function sinceDay(stamp: string, now: Date): string {
  const at = toDate(stamp);
  if (sameLocalDay(at, now)) return 'today';
  if (isYesterday(at, now)) return 'yesterday';
  const day = `${at.getDate()} ${MONTHS_IN_FULL[at.getMonth()]}`;
  return at.getFullYear() === now.getFullYear() ? day : `${day} ${at.getFullYear()}`;
}

// The day the reader is in, not the day UTC is in.
export function sameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}
