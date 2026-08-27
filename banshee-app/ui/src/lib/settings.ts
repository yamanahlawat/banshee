import { writable, type Writable } from 'svelte/store';

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
