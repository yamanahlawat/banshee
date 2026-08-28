import { writable, type Writable } from 'svelte/store';

export type Job = 'Microphone' | 'Hotkey' | 'Voice' | 'Agents' | 'Setup' | 'More settings';
export const JOBS: Job[] = ['Microphone', 'Hotkey', 'Voice', 'Agents', 'Setup', 'More settings'];

// The jobs that have a panel to open. The rest are strip rows only.
export const OPENABLE: Job[] = ['Microphone', 'Hotkey', 'Voice', 'Agents', 'More settings'];

// The panels the window wraps in a band of its own. History brings its own
// chrome and takes the whole space above the strip.
export const BANDED: Job[] = ['Microphone', 'Hotkey', 'Voice', 'Agents'];

// One job stands open at a time, so opening one closes the other.
export const open: Writable<Job | null> = writable(null);
