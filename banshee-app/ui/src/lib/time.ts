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

// The day the reader is in, not the day UTC is in.
export function sameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
  );
}
