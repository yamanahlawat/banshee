import { get } from 'svelte/store';
import { beforeEach, expect, it } from 'vitest';
import { forgetSamples, record, samples } from './downloads';
import { paceLine, paceOf } from './pace';
import type { Progress } from './daemon';

const MiB = 1_048_576;

const p = (bytes: number, index = 1, model = 'a.bin', total: number | null = 100): Progress => ({
  model,
  index,
  count: 4,
  bytes,
  total,
  state: 'downloading',
});

/// The pace after one file's samples, each an [at, bytes] pair.
function paceAfter(points: [number, number][], total: number | null = 25 * MiB) {
  for (const [at, bytes] of points) record(p(bytes, 1, 'a.bin', total), at);
  return paceOf(get(samples));
}

beforeEach(forgetSamples);

it('waits for a third sample of a file', () => {
  expect(
    paceAfter([
      [0, 0],
      [1, 10],
    ]),
  ).toBeNull();
});

it('starts the span at the second sample', () => {
  expect(
    paceAfter([
      [0, 0],
      [5, MiB],
      [2005, 5 * MiB],
    ]),
  ).toEqual({
    bytesPerSecond: 2 * MiB,
    secondsLeft: 10,
  });
});

it('ends the span at the latest sample', () => {
  expect(
    paceAfter([
      [0, 0],
      [5, MiB],
      [1005, 2 * MiB],
      [2005, 5 * MiB],
    ])?.bytesPerSecond,
  ).toBe(2 * MiB);
});

it('measures a resumed file from its first deltas', () => {
  const from = 500 * MiB;
  expect(
    paceAfter(
      [
        [0, from],
        [5, from + MiB],
        [2005, from + 5 * MiB],
      ],
      600 * MiB,
    )?.bytesPerSecond,
  ).toBe(2 * MiB);
});

it('gives no time left without a total', () => {
  expect(
    paceAfter(
      [
        [0, 0],
        [5, MiB],
        [2005, 5 * MiB],
      ],
      null,
    )?.secondsLeft,
  ).toBeNull();
});

it('gives no time left once bytes reach the total', () => {
  expect(
    paceAfter(
      [
        [0, 0],
        [5, MiB],
        [2005, 2 * MiB],
      ],
      MiB,
    )?.secondsLeft,
  ).toBeNull();
});

it('says nothing for a zero interval', () => {
  expect(
    paceAfter([
      [0, 0],
      [5, MiB],
      [5, 5 * MiB],
    ]),
  ).toBeNull();
});

it('says nothing for a negative interval', () => {
  expect(
    paceAfter([
      [0, 0],
      [5, MiB],
      [1, 5 * MiB],
    ]),
  ).toBeNull();
});

it('says nothing when no bytes arrived', () => {
  expect(
    paceAfter([
      [0, 0],
      [5, MiB],
      [2005, MiB],
    ]),
  ).toBeNull();
});

it.each([
  [2 * MiB, 10, '2.0 MB/s · less than a minute left'],
  [655_360, 240, '640 KB/s · about 4 min left'],
  [MiB, 4800, '1.0 MB/s · about 1 h 20 min left'],
  [MiB, null, '1.0 MB/s'],
  [400, null, 'under 1 KB/s'],
  [1000, 30, 'under 1 KB/s · less than a minute left'],
  [1024, null, '1 KB/s'],
])('words a pace of %s B/s and %s s left', (rate, left, line) => {
  expect(paceLine({ bytesPerSecond: rate, secondsLeft: left })).toBe(line);
});

it('words no pace as nothing', () => {
  expect(paceLine(null)).toBe('');
});

it.each([
  ['the next index', 2, 'a.bin'],
  ['another model', 1, 'b.bin'],
])('starts afresh on %s', (_, index, model) => {
  record(p(1), 0);
  record(p(2), 5);
  record(p(3), 1005);
  record(p(4, index, model), 2000);
  record(p(5, index, model), 2005);
  expect(paceOf(get(samples))).toBeNull();
  record(p(15, index, model), 3005);
  expect(paceOf(get(samples))?.bytesPerSecond).toBe(10);
});

it('starts afresh when the bytes go backwards', () => {
  record(p(50), 0);
  record(p(60), 5);
  record(p(70), 1005);
  record(p(10), 2000);
  record(p(20), 2005);
  expect(paceOf(get(samples))).toBeNull();
  record(p(30), 3005);
  expect(paceOf(get(samples))?.bytesPerSecond).toBe(10);
});

it('forgets the samples', () => {
  paceAfter([
    [0, 0],
    [5, MiB],
    [2005, 5 * MiB],
  ]);
  forgetSamples();
  expect(paceOf(get(samples))).toBeNull();
});
