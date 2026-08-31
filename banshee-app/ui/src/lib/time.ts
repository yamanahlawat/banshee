// Every row the daemon answers names its zone.
export function toDate(stamp: string): Date {
  return new Date(stamp);
}

// Local clock time, 24-hour, matching the window's mono time column.
export function formatTime(stamp: string): string {
  const d = toDate(stamp);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

// A clock time alone reads as today. One minute after midnight the newest
// dictation is yesterday's, and it must not say 23:58 and nothing more.
export function formatWhen(stamp: string, now: Date): string {
  const at = toDate(stamp);
  if (sameLocalDay(at, now)) return formatTime(stamp);
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  if (sameLocalDay(at, yesterday)) return `Yesterday ${formatTime(stamp)}`;
  return `${at.getDate()} ${MONTHS[at.getMonth()]} ${formatTime(stamp)}`;
}

// The day the reader is in, not the day UTC is in.
export function sameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}
