/// What a dashed mark says to anyone listening, in one place, so the wording
/// cannot differ between the marks that carry it.
export const RESTART_SAYS = '— set, and in effect when Banshee restarts';

/// The same fact as a sentence, for a row or a sub-row that has the width to
/// say it in full.
export const PENDING_SAYS = 'Set. It takes effect when Banshee restarts.';

// The whole group waits on one restart, so a group states it once.
export const TAKES_EFFECT = 'Your choice takes effect when Banshee restarts.';

import { writable } from 'svelte/store';
import { copyText } from './tauri';
import type { Listening, Speech } from './daemon';

// A side the daemon cannot name still gets a whole sentence.
export const A_SERVER = 'a remote server';

export function listeningLead(facts: Listening): string {
  if (!facts.live) return 'Banshee is not running, so no microphone is open.';
  if (facts.remote) {
    const host = facts.host ?? A_SERVER;
    if (facts.stoppedBy === 'keyfile') return `Banshee cannot read the key file for ${host}.`;
    if (facts.stoppedBy !== null) return `Banshee cannot reach ${host} to hear you.`;
    if (facts.pending) return `Banshee sends what you say to ${host} until it restarts.`;
    return `Banshee sends what you say to ${host} to be heard.`;
  }
  if (facts.pending) {
    return `Banshee will send what you say to ${facts.willUse ?? A_SERVER} when it restarts.`;
  }
  if (facts.device) return `Banshee is listening through the ${facts.device}.`;
  return facts.stoppedBy !== null
    ? 'Banshee cannot open a microphone.'
    : 'Banshee is not listening yet.';
}

export function listeningNote(facts: Listening): string {
  if (!facts.live) return 'Nothing is heard until Banshee starts.';
  const host = facts.host ?? A_SERVER;
  const waits = facts.pending ? ` ${TAKES_EFFECT}` : '';
  if (facts.remote) {
    if (facts.stoppedBy === 'keyfile') {
      return `Nothing goes to ${host} until the key file is removed and the key is set again.`;
    }
    if (facts.stoppedBy !== null) return `Nothing goes to ${host} until the listener starts.`;
    if (facts.pending) return `Audio still goes to ${host}.${waits}`;
    return `Audio goes to ${host}.`;
  }
  if (facts.pending) return `Audio still stays on this machine.${waits}`;
  return facts.keyPresent
    ? 'Audio stays on this machine. The server and key you set are still saved.'
    : 'Audio stays on this machine.';
}

// A started speaker's name comes off the live table, which a write the restart
// has not applied may have cleared, so the host carries the sentence and the
// name only decorates it.
export function speechLead(facts: Speech): string {
  if (!facts.live) return 'Banshee is not running, so nothing is spoken.';
  const host = facts.host ?? A_SERVER;
  if (!facts.remote) {
    if (facts.pending) {
      return `Banshee will speak through ${facts.willUse ?? A_SERVER} when it restarts.`;
    }
    if (!facts.started) {
      return `Banshee speaks with the system voice. ${facts.voiceName || 'The local voice'} did not load.`;
    }
    return facts.voiceName ? `Banshee speaks as ${facts.voiceName}.` : 'Banshee has no voice yet.';
  }
  if (!facts.started) {
    // The reader has already chosen the other speaker, so the fields that would
    // fix this one are no longer the thing to name.
    if (facts.pending) {
      return `Banshee cannot speak through ${host}. Your choice takes effect when it restarts.`;
    }
    if (!facts.keyPresent) return `Banshee cannot speak through ${host} until you paste a key.`;
    if (!facts.voiceName) return `Banshee cannot speak through ${host} until you name a voice.`;
    return `Banshee cannot speak through ${host}.`;
  }
  if (facts.pending) return `Banshee speaks through ${host} until it restarts.`;
  return facts.voiceName
    ? `Banshee speaks through ${host} as ${facts.voiceName}.`
    : `Banshee speaks through ${host}.`;
}

// A speaker the daemon did not build speaks nothing, so the note may not say
// the text leaves. The key row and the voice row name the fix.
export function speechNote(facts: Speech): string {
  if (!facts.live) return 'Nothing is spoken until Banshee starts.';
  const host = facts.host ?? A_SERVER;
  const waits = facts.pending ? ` ${TAKES_EFFECT}` : '';
  if (facts.remote && !facts.started) {
    return `The speaker on ${host} did not start, so text stays on this machine.${waits}`;
  }
  if (facts.pending) {
    return facts.remote
      ? `Text still goes to ${host}.${waits}`
      : `Text still stays on this machine.${waits}`;
  }
  if (facts.remote) return `Text goes to ${host}.`;
  return facts.keyPresent
    ? 'Text stays on this machine. The server and key you set are still saved.'
    : 'Text stays on this machine.';
}

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

// Announces a value that changed, and never the first value it is given: what
// already stood when a surface opened did not just arrive, and the reader who
// is not looking at the screen is the one this speaks to.
export function announcer<T>(): (value: T, says: string) => void {
  let seen: T | undefined;
  return (value, says) => {
    if (seen !== undefined && says !== '' && value !== seen) announce(says);
    seen = value;
  };
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
