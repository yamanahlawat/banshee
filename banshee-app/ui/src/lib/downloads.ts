import { derived, get, writable } from 'svelte/store';
import { daemon, type Progress } from './daemon';
import type { Samples } from './pace';
import { downloadModels } from './tauri';
import { announce } from './copy';

// `download_models` answers as soon as the daemon takes the task, and the first
// progress push waits on an HTTP handshake, so the reply is no sign a run has
// begun. The daemon refuses a second one, and the refusal reads as a failure.
const asked = writable(false);

/// Every control that starts a run reads this, so a press on one stops them all.
export const fetching = derived(
  [asked, daemon],
  ([$asked, $daemon]) => $asked || $daemon.download !== null,
);

/// Synchronous on purpose: a caller that awaits anything before claiming leaves
/// a second press the whole of that wait to pass through.
export function claimTheRun(): boolean {
  if (get(fetching)) return false;
  asked.set(true);
  return true;
}

/// Called on a push, and by a suite between renders: this outlives a component.
export function forgetTheAsk(): void {
  asked.set(false);
}

export const ONE_AT_A_TIME =
  'One download at a time. This works again when the current one finishes.';
export const BUSY_NOTE = 'busy-note';

const pressed = writable<string | null>(null);

/// The button pressed while a run held the download, until the run ends.
export const blocked = { subscribe: pressed.subscribe };

fetching.subscribe((running) => {
  if (!running) pressed.set(null);
});

/// Called when the note goes with its page, as a refusal does.
export function forgetBlocked(button: string): void {
  if (get(pressed) === button) pressed.set(null);
}

/// Claims the run for one button. Refused, that button is the blocked one and
/// the reason is said once.
export function claimFor(button: string): boolean {
  if (claimTheRun()) return true;
  pressed.set(button);
  announce(ONE_AT_A_TIME);
  return false;
}

export async function runTheDownload(): Promise<void> {
  try {
    await downloadModels();
  } catch (error) {
    forgetTheAsk();
    throw error;
  }
}

export async function askForModels(button: string): Promise<void> {
  if (!claimFor(button)) return;
  await runTheDownload();
}

const taken = writable<Samples | null>(null);

export const samples = { subscribe: taken.subscribe };

/// One file's samples: the next file, or bytes that fall back, start afresh.
export function record(progress: Progress, at: number): void {
  const { model, index, bytes, total } = progress;
  const sample = { bytes, total, at };
  taken.update((run) => {
    const same =
      run !== null && run.model === model && run.index === index && bytes >= run.latest.bytes;
    if (!same) return { model, index, taken: 1, second: null, latest: sample };
    const count = run.taken + 1;
    return { ...run, taken: count, second: count === 2 ? sample : run.second, latest: sample };
  });
}

export function forgetSamples(): void {
  taken.set(null);
}
