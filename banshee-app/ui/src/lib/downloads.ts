import { derived, get, writable } from 'svelte/store';
import { daemon } from './daemon';
import { downloadModels } from './tauri';

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

export async function runTheDownload(): Promise<void> {
  try {
    await downloadModels();
  } catch (error) {
    forgetTheAsk();
    throw error;
  }
}

export async function askForModels(): Promise<void> {
  if (!claimTheRun()) return;
  await runTheDownload();
}
