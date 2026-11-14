import { get, writable, type Readable } from 'svelte/store';
import { daemon, markPending, refreshStatus } from './daemon';
import { setSetting, settingsTable, status, type Setting } from './tauri';
import { announce, CHOICE_LABELS, refusalSays } from './copy';

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

const refusals = writable(new Map<string, string>());

/// What each refused key's row says, until that key is written again.
export const refused: Readable<Map<string, string>> = { subscribe: refusals.subscribe };

export function refusalId(key: string): string {
  return `refused-${key.replaceAll('.', '-')}`;
}

export function refuse(
  key: string,
  message: string,
  say: (message: string) => void = announce,
): void {
  refusals.update((held) => new Map(held).set(key, message));
  say(message);
}

export function forgetRefusal(key: string): void {
  const held = get(refusals);
  if (!held.has(key)) return;
  const kept = new Map(held);
  kept.delete(key);
  refusals.set(kept);
}

/// One more on every forget, so a refusal from a write made before it is said
/// and not kept.
let generation = 0;

export function forgetRefusals(): void {
  generation += 1;
  if (get(refusals).size > 0) refusals.set(new Map());
}

const latestWrite = new Map<string, number>();

// Every caller is a control the user just moved, so a refusal is said rather
// than left as a rejected promise nothing reads.
export async function write(
  key: string,
  value: unknown,
  say: (message: string) => void = announce,
): Promise<boolean> {
  const mine = (latestWrite.get(key) ?? 0) + 1;
  latestWrite.set(key, mine);
  const since = generation;
  forgetRefusal(key);
  try {
    await set(key, value);
    return true;
  } catch (error) {
    if (latestWrite.get(key) !== mine) return false;
    if (since === generation) refuse(key, refusalSays(error), say);
    else say(refusalSays(error));
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
