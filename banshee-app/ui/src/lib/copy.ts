import { writable } from 'svelte/store';
import { copyText } from './tauri';

export const copied = writable<string | null>(null);
export const announcement = writable('');

let timer: ReturnType<typeof setTimeout> | undefined;
const HELD_MS = 1500;

// The timer below outlives the copy that armed it, so anything resetting
// these stores has to disarm it too.
export function forgetCopy(): void {
  clearTimeout(timer);
  copied.set(null);
  announcement.set('');
}

export function announce(message: string): void {
  clearTimeout(timer);
  announcement.set(message);
  timer = setTimeout(() => {
    copied.set(null);
    announcement.set('');
  }, HELD_MS);
}

export async function copy(text: string, id: string): Promise<void> {
  // A live region speaks a change, not a value, so the same word twice is
  // silent. The clipboard round trip is the yield that clears it first.
  announcement.set('');
  try {
    await copyText(text);
  } catch {
    // Saying nothing would read as a copy that worked.
    announcement.set('Copy failed');
    return;
  }
  copied.set(id);
  announcement.set('Copied');
  clearTimeout(timer);
  timer = setTimeout(() => {
    copied.set(null);
    announcement.set('');
  }, HELD_MS);
}
