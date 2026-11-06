import { readable, type Readable } from 'svelte/store';

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

// The day the reader is in, not the day UTC is in.
export function sameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/// The moment the reader's day last began to be shown, for what a day label
/// reads. Checked each minute rather than timed to midnight: a long timer is not
/// promised to fire on time once the machine has slept.
export const today: Readable<Date> = readable(new Date(), (set) => {
  let shown = new Date();
  set(shown);
  const timer = setInterval(() => {
    const now = new Date();
    if (!sameLocalDay(now, shown)) set((shown = now));
  }, 60_000);
  return () => clearInterval(timer);
});
