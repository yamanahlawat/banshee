import { writable, type Writable } from 'svelte/store';
import { detectAgents, type AgentRow } from './tauri';

export const agents: Writable<AgentRow[]> = writable([]);

// Answers whether the read landed rather than throwing, so a dead daemon does
// not take the status beside it down. An empty list is a real answer.
export async function refresh(): Promise<boolean> {
  try {
    agents.set(await detectAgents());
    return true;
  } catch {
    // A stale row is a smaller wrong than an empty list.
    return false;
  }
}
