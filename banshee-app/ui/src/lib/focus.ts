import { tick } from 'svelte';

/// Moves focus after the DOM has caught up with the state change that destroyed
/// the control the user just pressed. Without the move focus falls to the body,
/// and a reader who cannot see the screen tabs from the top of the document to
/// reach what they asked for. axe cannot see this, so the tests assert it.
export async function land(on: () => HTMLElement | null | undefined): Promise<void> {
  await tick();
  on()?.focus();
}
