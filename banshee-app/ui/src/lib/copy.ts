/// What a dashed mark says to anyone listening, in one place, so the wording
/// cannot differ between the marks that carry it.
export const RESTART_SAYS = '— set, and in effect when Banshee restarts';

/// The same fact as a sentence, for a row or a sub-row that has the width to
/// say it in full.
export const PENDING_SAYS = 'Set. It takes effect when Banshee restarts.';

/// What each side calls its own failure. The server's words follow, so these
/// end in a full stop rather than a colon.
export const SPEECH_FAILED = 'The last spoken reply failed.';
export const DICTATION_FAILED = 'The last dictation failed.';

/// The element that carries each one, named by the panel's group and by the
/// header notice that lands the reader on it.
export const SPEECH_FAILURE = 'speech-failure';
export const DICTATION_FAILURE = 'dictation-failure';

/// The whole failure as one line, for the window to speak and for Copy to hand
/// over. The panel draws the two parts itself.
export function failureSays(label: string, said: string): string {
  return `${label} ${said}`;
}

/// The same failure for the eye, as one sentence. The daemon's words arrive
/// lower case and unterminated, so the label gives up its full stop.
export function failureReads(label: string, said: string): string {
  return `${label.replace(/\.+$/, '')}: ${said.trim().replace(/\.+$/, '')}.`;
}

/// The languages a listening model hears: the names up to three, and a count
/// beyond that.
export function languagesHeard(codes: string[]): string {
  if (codes.length > 3) return `${codes.length} languages`;
  const names = new Intl.DisplayNames(['en'], { type: 'language' });
  return new Intl.ListFormat('en', { type: 'conjunction' }).format(
    codes.map((code) => names.of(code) ?? code),
  );
}

/// What a listening model's row states, beside its name. Only the macOS chip
/// shows words as they are heard, so elsewhere streaming goes unsaid.
export function modelFacts(
  model: Pick<ListeningModel, 'megabytes' | 'languages' | 'streams'>,
  mac: boolean,
): string {
  const facts = `${downloadSize(model.megabytes)}. ${languagesHeard(model.languages)}.`;
  if (!mac) return facts;
  return `${facts} ${model.streams ? 'Shows words as you speak.' : 'Shows words once you stop.'}`;
}

/// Why a chosen model's download waits: the window cannot accept a licence.
export function licenceWait(model: Pick<ListeningModel, 'label' | 'licence'>): string {
  return `Banshee will not download ${model.label} until you accept its licence, ${model.licence.name}.`;
}

/// A failure says where the licence lives, so the reader can still open it.
export function readLicence(model: Pick<ListeningModel, 'id' | 'label'>): void {
  openLicence(model.id).catch((error: { message?: string }) =>
    report(error?.message || `The licence for ${model.label} did not open.`),
  );
}

/// Only the daemon knows which files are already here, so the window never sums a cost itself.
export function downloadSize(megabytes: number): string {
  return megabytes >= 1000 ? `${(megabytes / 1000).toFixed(1)} GB` : `${megabytes} MB`;
}

/// What a model the reader just chose costs. `owed` is the whole run, which
/// brings the detector and a voice as well, so it is named only when it is more
/// than the one file.
export function modelCost(chosen: string, megabytes: number, owed: number): string {
  const one = `${chosen} is not on this machine. About ${downloadSize(megabytes)} to fetch.`;
  return owed > megabytes
    ? `${one} Download brings everything missing, ${downloadSize(owed)} in all.`
    : one;
}

// The whole group waits on one restart, so a group states it once.
export const TAKES_EFFECT = 'Your choice takes effect when Banshee restarts.';

import { writable } from 'svelte/store';
import { copyText, openLicence } from './tauri';
import type { Listening, ListeningModel, Speech } from './daemon';

// A side the daemon cannot name still gets a whole sentence.
export const A_TRANSCRIPTION_SERVER = 'a transcription server';
export const A_VOICE_SERVER = 'a voice server';

export function listeningLead(facts: Listening): string {
  if (!facts.live) return 'Banshee is not running, so no microphone is open.';
  if (facts.remote) {
    const host = facts.host ?? A_TRANSCRIPTION_SERVER;
    if (facts.stoppedBy === 'keyfile') return `Banshee cannot read the key file for ${host}.`;
    if (facts.stoppedBy !== null) return `Banshee cannot reach ${host} to hear you.`;
    if (facts.pending) return `Banshee sends what you say to ${host} until it restarts.`;
    return `Banshee sends what you say to ${host} to be heard.`;
  }
  if (facts.pending) {
    return `Banshee will send what you say to ${facts.willUse ?? A_TRANSCRIPTION_SERVER} when it restarts.`;
  }
  if (facts.device) return `Banshee is listening through the ${facts.device}.`;
  return facts.stoppedBy !== null
    ? 'Banshee cannot open a microphone.'
    : 'Banshee is not listening yet.';
}

export function listeningNote(facts: Listening): string {
  if (!facts.live) return 'Nothing is heard until Banshee starts.';
  const host = facts.host ?? A_TRANSCRIPTION_SERVER;
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
  const host = facts.host ?? A_VOICE_SERVER;
  if (!facts.remote) {
    if (facts.pending) {
      return `Banshee will speak through ${facts.willUse ?? A_VOICE_SERVER} when it restarts.`;
    }
    if (!facts.started) {
      const fix = facts.voiceDownloaded
        ? 'Pick another voice.'
        : 'Get its files below, or pick another voice.';
      return `${facts.voiceName || 'The local voice'} did not load, so Banshee is silent. ${fix}`;
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
  const host = facts.host ?? A_VOICE_SERVER;
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
