import { MEGABYTE } from './daemon';

export type Sample = { bytes: number; total: number | null; at: number };

export type Samples = {
  model: string;
  index?: number;
  taken: number;
  second: Sample | null;
  latest: Sample;
};

export type Pace = { bytesPerSecond: number; secondsLeft: number | null };

// The daemon reports once before its read loop and again on the first chunk,
// milliseconds apart, so the span starts at the second sample.
export function paceOf(samples: Samples | null): Pace | null {
  if (samples === null || samples.taken < 3 || samples.second === null) return null;
  const { second: start, latest: last } = samples;
  const seconds = (last.at - start.at) / 1000;
  const gained = last.bytes - start.bytes;
  if (seconds <= 0 || gained <= 0) return null;
  const bytesPerSecond = gained / seconds;
  const secondsLeft =
    last.total !== null && last.total > last.bytes
      ? (last.total - last.bytes) / bytesPerSecond
      : null;
  return { bytesPerSecond, secondsLeft };
}

function left(seconds: number): string {
  if (seconds < 60) return 'less than a minute left';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `about ${minutes} min left`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `about ${hours} h left` : `about ${hours} h ${rest} min left`;
}

export function paceLine(pace: Pace | null): string {
  if (pace === null) return '';
  const rate =
    pace.bytesPerSecond >= MEGABYTE
      ? `${(pace.bytesPerSecond / MEGABYTE).toFixed(1)} MB/s`
      : pace.bytesPerSecond >= 1024
        ? `${Math.round(pace.bytesPerSecond / 1024)} KB/s`
        : 'under 1 KB/s';
  return pace.secondsLeft === null ? rate : `${rate} · ${left(pace.secondsLeft)}`;
}
