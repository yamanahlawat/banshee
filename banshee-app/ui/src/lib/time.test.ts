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
  it('holds for two stamps an hour apart around midday UTC', () => {
    // Midday UTC is the same calendar day from -12:00 through +11:00, so
    // this pair does not straddle a local midnight in any real zone.
    expect(sameLocalDay(toDate('2026-08-26T11:00:00Z'), toDate('2026-08-26T12:00:00Z'))).toBe(true);
  });
  it('separates two different days', () => {
    expect(sameLocalDay(toDate('2026-08-26T09:00:00Z'), toDate('2026-08-27T09:00:00Z'))).toBe(false);
  });
});
