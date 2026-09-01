/// What a dashed mark says to anyone listening, in one place, so the wording
/// cannot differ between the marks that carry it.
export const RESTART_SAYS = '— set, and in effect when Banshee restarts';

import { writable } from 'svelte/store';
import { copyText } from './tauri';

/// A sentence should not open on a digit, and a count mid-sentence reads better
/// as a word. Past what this names, the digit is clearer than the word anyway.
const WORDS = ['no', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine'];

export function spell(n: number, capital = false): string {
  const word = WORDS[n] ?? String(n);
  return capital ? word.charAt(0).toUpperCase() + word.slice(1) : word;
}

export const copied = writable<string | null>(null);
export const announcement = writable('');

/// A confirmation may expire, because the reader either saw it or did not need
/// it. A failure may not: the reader is often not looking at the screen at all.
/// So it holds until dismissed.
export const problem = writable('');

/// Announced from the element that draws it rather than from a second hidden
/// copy, so a screen reader hears it once and finds it where it was spoken.
export function report(message: string): void {
  problem.set(message);
}

let timer: ReturnType<typeof setTimeout> | undefined;
const HELD_MS = 1500;

// The timer below outlives the copy that armed it, so anything resetting
// these stores has to disarm it too.
export function forgetCopy(): void {
  clearTimeout(timer);
  copied.set(null);
  announcement.set('');
  problem.set('');
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
    // Saying nothing would read as a copy that worked: the button still says
    // Copy either way.
    report('Nothing was copied. The clipboard refused it.');
    return;
  }
  copied.set(id);
  problem.set('');
  announcement.set('Copied');
  clearTimeout(timer);
  timer = setTimeout(() => {
    copied.set(null);
    announcement.set('');
  }, HELD_MS);
}
