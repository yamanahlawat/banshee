import { writable } from 'svelte/store';
import { copyText } from './tauri';

export const copied = writable<string | null>(null);
export const announcement = writable('');

let timer: ReturnType<typeof setTimeout> | undefined;

export async function copy(text: string, id: string): Promise<void> {
  await copyText(text);
  copied.set(id);
  announcement.set('Copied');
  clearTimeout(timer);
  timer = setTimeout(() => copied.set(null), 1500);
}
