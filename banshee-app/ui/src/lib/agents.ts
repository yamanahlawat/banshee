import { writable, type Writable } from 'svelte/store';
import { detectAgents, type AgentRow } from './tauri';

// The strip counts what is connected and the panel lists it, so one read
// serves both and a connect made in the panel moves the count below it.
export const agents: Writable<AgentRow[]> = writable([]);

// A dead daemon must not take the status and the history beside it down.
export async function refresh(): Promise<void> {
  try {
    agents.set(await detectAgents());
  } catch {
    // A failed read after a landed connect leaves a stale row, which is a
    // smaller wrong than an empty list.
  }
}
