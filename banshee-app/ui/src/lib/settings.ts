import { writable, type Writable } from 'svelte/store';
import { daemon, markPending, reduceStatus } from './daemon';
import { setSetting, status } from './tauri';
import { announce } from './copy';

const key = 'banshee.showCommands';

function read(): boolean {
  try {
    return localStorage.getItem(key) === 'true';
  } catch {
    return false;
  }
}

export const showCommands: Writable<boolean> = writable(read());

showCommands.subscribe((value) => {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    // localStorage can be unavailable; the store still holds the value in memory.
  }
});

// Every caller is a control the user just moved, so a refusal has to be said
// rather than left as a rejected promise nothing reads. A caller with a place
// of its own to say it passes one.
export async function write(
  key: string,
  value: unknown,
  say: (message: string) => void = announce,
): Promise<void> {
  try {
    await set(key, value);
  } catch (error) {
    say((error as { message?: string })?.message || 'The daemon refused that.');
  }
}

export async function set(key: string, value: unknown): Promise<void> {
  // The daemon answers which keys it could not apply live, and those are the
  // ones a row marks pending.
  const restartRequired = await setSetting(key, value);
  daemon.update((state) => markPending(state, restartRequired));
  // A write changes what `status` answers, and nothing pushes that, so a row
  // would go on showing the value the user just replaced.
  try {
    const fresh = await status();
    daemon.update((state) => reduceStatus(state, fresh));
  } catch {
    // The write landed. A stale row is a smaller wrong than a lost setting.
  }
}
