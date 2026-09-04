import { derived, writable } from 'svelte/store';
/// `remedy` is what clears it and `role` is which file it names. `kind` says
/// which part is at fault and `command` is the line a person could run;
/// neither answers what a client has to route on. Both are absent from a
/// daemon older than them.
export type Remedy = 'download' | 'restart' | 'grant';
export type FileRole = 'speech' | 'detector' | 'engine' | 'voice';
export type Blocker = {
  kind: string;
  id: string;
  name: string;
  role?: FileRole;
  remedy?: Remedy;
  consequence: string;
  fix: string;
  command?: string;
};
export type Status = Record<string, unknown> & {
  english_only?: boolean;
  download_megabytes?: number;
  running: boolean;
  blockers?: Blocker[];
  config?: Record<string, Record<string, unknown>>;
  pending?: string[];
  history_enabled?: boolean;
};
export type Live = {
  recording: boolean;
  speaking: boolean;
  armed: boolean;
  transcribing: boolean;
  audio_device: string | null;
  missing_device: string | null;
};
export type Daemon = {
  status: Status | null;
  live: Live;
  pending: Set<string>;
  down: string | null;
  download: Progress | null;
};
export type Progress = {
  label?: string;
  model: string;
  index?: number;
  count?: number;
  bytes: number;
  total: number | null;
  state: 'downloading' | 'done' | 'failed';
};
export type Word =
  | 'Ready'
  | 'Recording'
  | 'Speaking'
  | 'Listening'
  | 'Working'
  | 'Not ready'
  | 'Downloading'
  | 'Not running';
export type LampForm = 'idle' | 'recording' | 'speaking' | 'listening' | 'notrunning';

export function empty(): Daemon {
  return {
    status: null,
    live: {
      recording: false,
      speaking: false,
      armed: false,
      transcribing: false,
      audio_device: null,
      missing_device: null,
    },
    pending: new Set(),
    down: null,
    download: null,
  };
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
  return {
    ...state,
    status,
    live: { ...state.live, ...liveFrom(status) },
    pending: new Set(status.pending ?? []),
    down: status.running === false ? (state.down ?? 'not running') : null,
  };
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
  if (state.download !== null) return 'Downloading';
  if ((state.status?.blockers?.length ?? 0) > 0) return 'Not ready';
  return 'Ready';
}
export function lampForm(word: Word): LampForm {
  if (word === 'Not running') return 'notrunning';
  if (word === 'Recording') return 'recording';
  if (word === 'Speaking') return 'speaking';
  // The one state where doing nothing is the wrong answer, so it cannot share
  // a silhouette with Ready. Working, Downloading and Not ready still do: each
  // resolves on its own, and the window shouts them in the body anyway.
  if (word === 'Listening') return 'listening';
  return 'idle';
}

/// The daemon writes these lower case, to sit after a colon in a terminal. The
/// window sets them as their own sentence, so it capitalises rather than asking
/// the daemon to write for two readers at once.
function sentence(prose: string): string {
  return prose.charAt(0).toUpperCase() + prose.slice(1);
}

// The daemon writes the same command twice, as prose and as a field.
export function fixProse(blocker: Blocker): string | null {
  if (blocker.command === undefined || !blocker.fix.endsWith(blocker.command)) {
    return sentence(blocker.fix);
  }
  const lead = blocker.fix.slice(0, -blocker.command.length);
  return /^(?:run|restart|start)(?: it)?:\s*$/.test(lead) ? null : sentence(blocker.fix);
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
  return open ?? 'Not open';
}

export function deviceLabel(live: string | null): string {
  return live ? `Default (${live})` : 'Default';
}

// The daemon holds these as f32, so 1.2 arrives as 1.2000000476837158.
export function shownFloat(value: number): number {
  return Math.round(value * 100) / 100;
}

export function percent(bytes: number, total: number | null): number | null {
  if (total === null || total <= 0) return null;
  return Math.min(100, Math.floor((bytes / total) * 100));
}

function names(progress: Progress): string {
  return progress.label && progress.count
    ? `${progress.label}, ${progress.index} of ${progress.count}`
    : progress.model;
}

export function downloadLine(progress: Progress): string {
  const done = percent(progress.bytes, progress.total);
  const named = names(progress);
  // The run carries on to the next file, so a failure that says nothing leaves
  // a person clicking Download again with no idea what went wrong.
  if (progress.state === 'failed') return `${named} · failed`;
  if (done === null) return `${named} · ${Math.round(progress.bytes / 1_048_576)} MB`;
  return `${named} · ${done}%`;
}

/// The daemon reports each percent, and a live region reads every change it is
/// given, so the line on screen and the line said aloud cannot be one string.
/// This one holds still between quarters.
const SPOKEN_STEP = 25;

export function spokenProgress(progress: Progress): string {
  if (progress.state === 'failed') return downloadLine(progress);
  const done = percent(progress.bytes, progress.total);
  const named = names(progress);
  // No length to measure against is no progress to say, so the file is named
  // and nothing further changes until the next one starts.
  if (done === null) return named;
  return `${named} · ${Math.floor(done / SPOKEN_STEP) * SPOKEN_STEP}%`;
}

/// The daemon blocks on two files and fetches four, so the blocking two land
/// while the rest are still coming: being unblocked is not being finished. A
/// daemon that sends no count has no last file to name, so any terminal report
/// ends it.
export function endsTheRun(progress: Progress): boolean {
  if (progress.state === 'downloading') return false;
  return !progress.count || progress.index === progress.count;
}

/// A key that refuses for want of a file is answered by the download that
/// brings it, and the daemon asks it again as the run ends, so naming a restart
/// while one is arriving is advice that cannot work.
const WAITS_ON_A_FILE = new Set(['stt.preset', 'tts.voice', 'tts.speed']);

export const waitsOnARestart = derived(daemon, (state) => {
  const fetching =
    state.download !== null ||
    (state.status?.blockers ?? []).some((blocker) => blocker.remedy === 'download');
  const keys = [...state.pending].filter((key) => !(fetching && WAITS_ON_A_FILE.has(key)));
  return new Set(keys);
});
