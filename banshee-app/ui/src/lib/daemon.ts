import { writable } from 'svelte/store';
export type Blocker = { kind: string; id: string; name: string; consequence: string; fix: string; command?: string };
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
  // Pending is the daemon's answer: it knows which keys it applied live.
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
// Which stations hold the pad shut. A download in progress still holds it,
// or the band would vanish under the button that started it. A microphone
// does not: the history it recorded is still worth showing.
export function setupBlocked(state: Daemon): boolean {
  return checklist(state).some(
    (row) =>
      (row.state === 'blocked' || row.state === 'working') && row.station !== 'Microphone',
  );
}

// The needle rests on the first station that is not clear. `Try it` is never
// clear, so it catches the needle on a machine with nothing left to fix.
export function needleAt(rows: { state: string }[]): number {
  const at = rows.findIndex((row) => row.state !== 'clear');
  return at === -1 ? rows.length - 1 : at;
}

// The daemon writes the same command twice, as prose and as a field. A
// sentence that only restates the command line below it is worth dropping.
export function fixProse(blocker: Blocker): string | null {
  if (blocker.command === undefined || !blocker.fix.endsWith(blocker.command)) return blocker.fix;
  const lead = blocker.fix.slice(0, -blocker.command.length);
  return /^(?:run|restart|start)(?: it)?:\s*$/.test(lead) ? null : blocker.fix;
}

// Every missing model is downloaded by one call, so the blockers that share
// that call belong under one row. A permission names its own pane.
export function fixGroups(blockers: Blocker[]): Blocker[][] {
  const groups = new Map<string, Blocker[]>();
  for (const blocker of blockers) {
    const key = blocker.kind === 'model' ? 'model' : blocker.id;
    groups.set(key, [...(groups.get(key) ?? []), blocker]);
  }
  return [...groups.values()];
}

function stationOf(blocker: Blocker): Station {
  if (blocker.kind === 'permission') return 'Permissions';
  if (blocker.kind === 'model') return 'Models';
  if (blocker.kind === 'pipeline') return 'Microphone';
  return 'Running';
}
export const daemon = writable<Daemon>(empty());

export const SYSTEM_DEVICE = 'default';

// The daemon names no device until it opens one, so this says which device
// its own word stands for.
export function deviceLabel(live: string | null): string {
  return live ? `Default (${live})` : 'Default';
}

// The daemon holds its float settings as f32, so 1.2 reaches a client as
// 1.2000000476837158. Every step it offers has one decimal or two.
export function shownFloat(value: number): number {
  return Math.round(value * 100) / 100;
}
