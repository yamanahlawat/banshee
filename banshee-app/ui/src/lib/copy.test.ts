import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('./tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));
import { copy, copied } from './copy';
beforeEach(() => vi.useFakeTimers());
it('marks the control Copied for a moment, then clears', async () => {
  await copy('hello', 'row-14:02');
  expect(get(copied)).toBe('row-14:02');
  vi.advanceTimersByTime(1500);
  expect(get(copied)).toBeNull();
});
