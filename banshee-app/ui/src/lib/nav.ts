import { WAITING_SAYS, waitingSays } from './copy';

export const MAIN = ['History', 'Agents'] as const;
export const SETTINGS = ['Listening', 'Speaking', 'Hotkey', 'Feedback', 'Saved history'] as const;
export const PAGES = [...MAIN, ...SETTINGS] as const;
export type Page = (typeof PAGES)[number];

// A table, or one key of it, and the page that writes it.
const WRITTEN_ON: [string, Page][] = [
  ['stt', 'Listening'],
  ['models', 'Listening'],
  ['audio.input_device', 'Listening'],
  ['audio.keep_open', 'Listening'],
  ['tts', 'Speaking'],
  ['audio.hotkey', 'Hotkey'],
  ['audio.hotkey_mode', 'Hotkey'],
  ['audio.barge_in', 'Hotkey'],
  ['feedback', 'Feedback'],
  ['daemon.save_history', 'Saved history'],
];

function pageWriting(key: string): Page | null {
  const found = WRITTEN_ON.find(([path]) => key === path || key.startsWith(`${path}.`));
  return found ? found[1] : null;
}

/// The sentence each page's item says for the keys it holds that the daemon has
/// not applied. A page holding both kinds says it could not use one yet: a file
/// arrives on its own, and that key needs the person.
export function waitingOn(keys: Iterable<string>): Partial<Record<Page, string>> {
  const says: Partial<Record<Page, string>> = {};
  for (const key of keys) {
    const page = pageWriting(key);
    if (page === null || says[page] === WAITING_SAYS) continue;
    says[page] = waitingSays(key);
  }
  return says;
}
