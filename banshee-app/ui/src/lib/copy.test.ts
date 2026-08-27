import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('./tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));
import { announcement, copy, copied } from './copy';
beforeEach(() => vi.useFakeTimers());
it('marks the control Copied for a moment, then clears', async () => {
  await copy('hello', 'row-14:02');
  expect(get(copied)).toBe('row-14:02');
  vi.advanceTimersByTime(1500);
  expect(get(copied)).toBeNull();
});

it('clears the announcement, so a second copy is a change the region speaks', async () => {
  await copy('hello', 'row-14:02');
  expect(get(announcement)).toBe('Copied');
  vi.advanceTimersByTime(1500);
  expect(get(announcement)).toBe('');
});

it('says so when the clipboard refuses, rather than looking like it worked', async () => {
  const { copyText } = await import('./tauri');
  vi.mocked(copyText).mockRejectedValueOnce(new Error('clipboard unavailable'));

  await copy('hello', 'row-14:02');

  expect(get(copied)).toBeNull();
  expect(get(announcement)).toBe('Copy failed');
});
