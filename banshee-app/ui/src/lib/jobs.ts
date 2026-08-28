import { writable, type Writable } from 'svelte/store';

export type Job = 'Microphone' | 'Hotkey' | 'Voice' | 'Agents' | 'Setup' | 'More settings';
export const JOBS: Job[] = ['Microphone', 'Hotkey', 'Voice', 'Agents', 'Setup', 'More settings'];

// The jobs that have a panel to open. The rest are strip rows only.
export const OPENABLE: Job[] = ['Microphone', 'Hotkey', 'Voice', 'Agents', 'More settings'];

// One job stands open at a time, so opening one closes the other.
export const open: Writable<Job | null> = writable(null);
