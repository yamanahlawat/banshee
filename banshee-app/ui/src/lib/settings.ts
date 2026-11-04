import { writable } from 'svelte/store';
import { daemon, markPending, refreshStatus } from './daemon';
import { setSetting, settingsTable, status, type Setting } from './tauri';
import { announce, CHOICE_LABELS } from './copy';

export type { Domain, Setting } from './tauri';

// The daemon's table of what each key takes and reads as when absent. The
// window keeps no copy of either.
export const table = writable<Setting[]>([]);

export async function loadTable(): Promise<void> {
  table.set(await settingsTable());
}

export function rowOf(rows: Setting[], key: string): Setting | undefined {
  return rows.find((row) => row.key === key);
}

/// The choices in the table's order, each under the window's word for it.
export function optionsOf(rows: Setting[], key: string): { value: string; label: string }[] {
  const domain = rowOf(rows, key)?.domain;
  const values = domain?.kind === 'choice' ? domain.values : [];
  return values.map((value) => ({
    value,
    label: CHOICE_LABELS[key]?.[value] ?? value,
  }));
}

export function defaultOf(rows: Setting[], key: string): unknown {
  const text = rowOf(rows, key)?.default;
  return text == null ? undefined : JSON.parse(text);
}

// Every caller is a control the user just moved, so a refusal is said rather
// than left as a rejected promise nothing reads.
export async function write(
  key: string,
  value: unknown,
  say: (message: string) => void = announce,
): Promise<boolean> {
  try {
    await set(key, value);
    return true;
  } catch (error) {
    say((error as { message?: string })?.message || 'The daemon refused that.');
    return false;
  }
}

export async function set(key: string, value: unknown): Promise<void> {
  const waiting = await setSetting(key, value);
  daemon.update((state) => markPending(state, waiting));
  // A write changes what `status` answers, and nothing pushes that, so a row
  // would go on showing the value the user just replaced.
  try {
    await refreshStatus(status);
  } catch {
    // The write landed. A stale row is a smaller wrong than a lost setting.
  }
}
