import { writable } from 'svelte/store';
export type Blocker = { kind: string; id: string; name: string; consequence: string; fix: string; command?: string };
export type Status = Record<string, unknown> & { running: boolean; blockers?: Blocker[]; config?: Record<string, Record<string, unknown>>; pending?: string[] };
export type Live = { recording: boolean; speaking: boolean; armed: boolean; transcribing: boolean; audio_device: string | null; missing_device: string | null };
export type Daemon = { status: Status | null; live: Live; pending: Set<string>; down: string | null; downloading: boolean };
export type Word = 'Ready' | 'Recording' | 'Speaking' | 'Listening' | 'Working' | 'Not ready' | 'Downloading' | 'Not running';
export type LampForm = 'idle' | 'recording' | 'speaking' | 'notrunning';

export function empty(): Daemon {
  return { status: null, live: { recording: false, speaking: false, armed: false, transcribing: false, audio_device: null, missing_device: null }, pending: new Set(), down: null, downloading: false };
}
const LIVE_KEYS = Object.keys(empty().live);

// The status reply carries the live flags at its top level, under the same
// names `daemon:state` pushes.
export function liveFrom(status: Status): Partial<Live> {
  const live: Record<string, unknown> = {};
  for (const key of LIVE_KEYS) {
    if (status[key] !== undefined) live[key] = status[key];
  }
  return live as Partial<Live>;
}

export function reduceStatus(state: Daemon, status: Status): Daemon {
  // The daemon reports which keys it applied live; the window does not guess.
  return { ...state, status, live: { ...state.live, ...liveFrom(status) }, pending: new Set(status.pending ?? []), down: status.running === false ? (state.down ?? 'not running') : null };
}
export function reduceLive(state: Daemon, live: Partial<Live>): Daemon {
  return { ...state, live: { ...state.live, ...live }, down: null };
}
export function markPending(state: Daemon, keys: string[]): Daemon {
  return { ...state, pending: new Set([...state.pending, ...keys]) };
}
// `recording` is true whenever `armed` is, so the narrower flag is tested first.
export function stateWord(state: Daemon): Word {
  if (state.down !== null || state.status?.running === false) return 'Not running';
  if (state.live.transcribing) return 'Working';
  if (state.live.armed) return 'Listening';
  if (state.live.recording) return 'Recording';
  if (state.live.speaking) return 'Speaking';
  if (state.downloading) return 'Downloading';
  if ((state.status?.blockers?.length ?? 0) > 0) return 'Not ready';
  return 'Ready';
}
export function lampForm(word: Word): LampForm {
  if (word === 'Not running') return 'notrunning';
  if (word === 'Recording') return 'recording';
  if (word === 'Speaking') return 'speaking';
  return 'idle';
}

// The daemon writes the same command twice, as prose and as a field.
export function fixProse(blocker: Blocker): string | null {
  if (blocker.command === undefined || !blocker.fix.endsWith(blocker.command)) return blocker.fix;
  const lead = blocker.fix.slice(0, -blocker.command.length);
  return /^(?:run|restart|start)(?: it)?:\s*$/.test(lead) ? null : blocker.fix;
}

// One call downloads every missing model, so those blockers share a row.
export function fixGroups(blockers: Blocker[]): Blocker[][] {
  const groups = new Map<string, Blocker[]>();
  for (const blocker of blockers) {
    const key = blocker.kind === 'model' ? 'model' : blocker.id;
    groups.set(key, [...(groups.get(key) ?? []), blocker]);
  }
  return [...groups.values()];
}

export const daemon = writable<Daemon>(empty());

export const SYSTEM_DEVICE = 'default';

// The configured name is deliberately not consulted: it is what was asked for,
// not what the daemon opened.
export function microphoneInUse(open: string | null): string {
  return open ?? 'No microphone';
}

export function deviceLabel(live: string | null): string {
  return live ? `Default (${live})` : 'Default';
}

// The daemon holds these as f32, so 1.2 arrives as 1.2000000476837158.
export function shownFloat(value: number): number {
  return Math.round(value * 100) / 100;
}
