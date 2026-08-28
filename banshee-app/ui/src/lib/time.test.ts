import { describe, expect, it } from 'vitest';
import { formatTime, sameLocalDay, toDate } from './time';

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
    expect(sameLocalDay(toDate('2026-08-26T09:00:00Z'), toDate('2026-08-27T09:00:00Z'))).toBe(false);
  });
});
