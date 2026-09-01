import { describe, expect, it } from 'vitest';
import { formatTime, formatWhen, sameLocalDay, toDate } from './time';

describe('formatTime', () => {
  // Two spellings of one instant must read alike. This holds under any host
  // zone, where a fixed clock string would not.
  it('reads an offset stamp as the instant it names', () => {
    expect(formatTime('2026-08-27T17:37:24+05:30')).toBe(formatTime('2026-08-27T12:07:24Z'));
  });

  it('keeps a zero offset', () => {
    expect(formatTime('2026-08-27T12:07:24+00:00')).toBe(formatTime('2026-08-27T12:07:24Z'));
  });

  it('pads to four digits', () => {
    expect(formatTime('2026-08-27T00:05:00Z')).toMatch(/^\d{2}:\d{2}$/);
  });
});

describe('sameLocalDay', () => {
  it('holds for two stamps an hour apart on one local day', () => {
    // Anchored to local noon. Midday UTC is local midnight at +12, which is
    // New Zealand, so a UTC pair straddles the day there.
    const noon = new Date(2026, 7, 26, 12, 0, 0);
    expect(sameLocalDay(new Date(noon.getTime() - 3_600_000), noon)).toBe(true);
  });
  it('separates two different days', () => {
    expect(sameLocalDay(toDate('2026-08-26T09:00:00Z'), toDate('2026-08-27T09:00:00Z'))).toBe(
      false,
    );
  });
});

describe('formatWhen', () => {
  const now = new Date(2026, 7, 29, 0, 3, 0);
  const at = (y: number, m: number, d: number, h: number, min: number) =>
    new Date(y, m, d, h, min).toISOString();

  it("gives the clock alone for a row from the reader's own day", () => {
    expect(formatWhen(at(2026, 7, 29, 0, 1), now)).toBe('00:01');
  });
  it('names yesterday, so 23:58 is not read as a minute ago', () => {
    expect(formatWhen(at(2026, 7, 28, 23, 58), now)).toBe('Yesterday 23:58');
  });
  it('dates anything older', () => {
    expect(formatWhen(at(2026, 7, 20, 9, 14), now)).toBe('20 Aug 09:14');
  });
  it('carries the year for a row from an earlier one', () => {
    expect(formatWhen(at(2025, 7, 20, 9, 14), now)).toBe('20 Aug 2025');
  });
  // The column is 52px, so the year must not make a third line.
  it('keeps a dated row to the width of one from this year', () => {
    const thisYear = formatWhen(at(2026, 7, 20, 9, 14), now);
    expect(formatWhen(at(2025, 7, 20, 9, 14), now).length).toBeLessThanOrEqual(thisYear.length);
  });
  it('counts back by the calendar, so a clock change cannot skip a day', () => {
    const firstOfMarch = new Date(2026, 2, 1, 0, 30, 0);
    expect(formatWhen(at(2026, 1, 28, 22, 0), firstOfMarch)).toBe('Yesterday 22:00');
  });
});
