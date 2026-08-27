import { writable } from 'svelte/store';
export type Blocker = { kind: string; id: string; name: string; consequence: string; fix: string };
export type Status = Record<string, unknown> & { running: boolean; blockers?: Blocker[]; config?: Record<string, Record<string, unknown>>; pending?: string[] };
export type Live = { recording: boolean; speaking: boolean; armed: boolean; transcribing: boolean; audio_device: string | null; missing_device: string | null };
export type Daemon = { status: Status | null; live: Live; pending: Set<string>; down: string | null; downloading: boolean };
export type Word = 'Ready' | 'Recording' | 'Speaking' | 'Listening' | 'Working' | 'Not ready' | 'Downloading' | 'Not running';
export type LampForm = 'idle' | 'recording' | 'speaking' | 'notrunning';
export type Station = 'Running' | 'Microphone' | 'Permissions' | 'Models' | 'Try it';
export const STATIONS: Station[] = ['Running', 'Microphone', 'Permissions', 'Models', 'Try it'];

export function empty(): Daemon {
  return { status: null, live: { recording: false, speaking: false, armed: false, transcribing: false, audio_device: null, missing_device: null }, pending: new Set(), down: null, downloading: false };
}
export function reduceStatus(state: Daemon, status: Status): Daemon {
  // Pending is the daemon's answer: it knows which keys it applied live.
  return { ...state, status, pending: new Set(status.pending ?? []), down: status.running === false ? (state.down ?? 'not running') : null };
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
export function checklist(state: Daemon) {
  const blockers = state.status?.blockers ?? [];
  const byStation = (station: Station) => blockers.filter((b) => stationOf(b) === station);
  return STATIONS.map((station) => {
    if (station === 'Try it') return { station, state: 'todo' as const, blockers: [] };
    if (station === 'Running' && state.down !== null) return { station, state: 'blocked' as const, blockers: [] };
    if (station === 'Models' && state.downloading) return { station, state: 'working' as const, blockers: [] };
    const own = byStation(station);
    return { station, state: own.length ? ('blocked' as const) : ('clear' as const), blockers: own };
  });
}
function stationOf(blocker: Blocker): Station {
  if (blocker.kind === 'permission') return 'Permissions';
  if (blocker.kind === 'model') return 'Models';
  if (blocker.kind === 'pipeline') return 'Microphone';
  return 'Running';
}
export const daemon = writable<Daemon>(empty());
