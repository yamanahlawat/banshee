// Reached only under `import.meta.env.DEV` and only when Tauri is absent, so
// it cannot ship.
import readyJson from './ready.json';
import permissionsJson from './permissions.json';
import notRunningJson from './not-running.json';
import remoteJson from './remote.json';
import remoteSpeechJson from './remote-speech.json';
import settingsJson from './settings.json';
import type { AgentRow, Devices, HistoryRow, PlannedChange, Voices } from '../lib/tauri';
import type { Status } from '../lib/daemon';

// A fixture is a captured reply, and JSON cannot carry the unions the type
// states, `Blocker['kind']` among them. Named once here, so every state built
// out of one below is still checked against `Status`.
const asStatus = (captured: unknown) => captured as Status;
const ready = asStatus(readyJson);
const permissions = asStatus(permissionsJson);
const notRunning = asStatus(notRunningJson);
const remote = asStatus(remoteJson);
const remoteSpeech = asStatus(remoteSpeechJson);

// A write changes what `status` answers next, as the daemon's would.
const written: Record<string, unknown> = {};

const HOST = 'api.openai.com';
const OPENAI = `https://${HOST}/v1`;

// What the daemon runs, beside a config that says only what was asked for. A
// side the daemon runs remotely always reaches this one host.
function inForce(
  listener: { remote: boolean; key: boolean },
  speaker: { remote: boolean; started: boolean; key: boolean },
): NonNullable<Status['remote']> {
  return {
    stt: {
      remote: listener.remote,
      host: listener.remote ? HOST : null,
      key_present: listener.key,
    },
    tts: {
      remote: speaker.remote,
      host: speaker.remote ? HOST : null,
      speaker_started: speaker.started,
      key_present: speaker.key,
    },
  };
}

// The speaker the config asks for, with the table a remote one reads. The
// table leaves out `response_format` and `sample_rate`, so the window shows
// their defaults.
function asks(provider: string, model: string, voice: string) {
  // Every fixture carries a config; the type leaves it optional because a
  // reply from a daemon that is not running does not.
  const base = remote.config ?? {};
  return {
    ...base,
    tts: {
      ...base.tts,
      provider,
      remote: { base_url: OPENAI, model, voice, instructions: '' },
    },
  };
}

const STATES: Record<string, Status> = {
  ready,
  permissions,
  remote,
  'remote-speech': remoteSpeech,
  'not-running': notRunning,
  'no-agents': ready,
  'copy-fails': ready,
  'retry-fails': ready,
  // A real first run: blocked, and nothing ever said.
  'first-run': permissions,
  downloading: permissions,
  // The header word and the mark read `activity`, which is the daemon's own
  // ranking of these flags, never the booleans beside it.
  recording: { ...ready, recording: true, activity: 'recording' },
  speaking: { ...ready, speaking: true, activity: 'speaking' },
  armed: { ...ready, recording: true, armed: true, activity: 'listening' },
  transcribing: { ...ready, transcribing: true, activity: 'busy' },
  // Parakeet is loaded and working; Whisper turbo has been chosen and is not
  // here. The daemon is not blocked, so this is the errand, not a fault.
  'needs-model': {
    ...ready,
    config: { ...ready.config, stt: { ...ready.config?.stt, model: 'whisper-large-v3-turbo' } },
    download_megabytes: 619,
    missing_downloads: [
      { name: 'whisper-large-v3-turbo-Q5_K_M.gguf', role: 'speech', megabytes: 619 },
    ],
  },
  // A model set through the CLI whose licence waits to be accepted, which the
  // window cannot do.
  'needs-licence': {
    ...ready,
    config: { ...ready.config, stt: { ...ready.config?.stt, model: 'canary-1b' } },
    stt_models: [
      ...(ready.stt_models ?? []),
      {
        id: 'canary-1b',
        label: 'canary-1b',
        megabytes: 1012,
        downloaded: false,
        suggested: false,
        languages: ['en', 'de', 'es', 'fr'],
        streams: false,
        credit: null,
        licence: {
          name: 'CC-BY-NC-4.0',
          summary: 'Use and share it, crediting its authors, for non-commercial purposes only.',
        },
        accept_command: 'banshee setup --model canary-1b --accept-licence',
      },
    ],
  },
  // A remote listener translates, so the Write it in row shows.
  translating: { ...remote, stt_translates: true },
  saving_off: { ...ready, config: { ...ready.config, daemon: { save_history: false } } },
  pending: { ...ready, pending: ['audio.hotkey', 'daemon.save_history'] },
  // The listener the config asks for and the one the daemon runs, in each
  // direction. Neither has any other way to be looked at: the pair only
  // disagrees when the listener asked for could not start.
  'to-remote': {
    ...remote,
    remote: inForce({ remote: false, key: true }, { remote: false, started: true, key: false }),
    pending: ['stt.provider'],
  },
  'to-local': {
    ...remote,
    config: { ...remote.config, stt: { ...remote.config?.stt, provider: 'local' } },
    pending: ['stt.provider'],
  },
  'no-key': {
    ...remote,
    remote: inForce({ remote: true, key: false }, { remote: false, started: true, key: false }),
  },
  'stt-failed': { ...remote, last_error: 'the remote listener refused the key' },
  // The speaker the config asks for and the one the daemon runs, in each
  // direction. Neither has any other way to be looked at.
  'to-remote-voice': {
    ...remote,
    config: asks('remote', 'gpt-4o-mini-tts', 'marin'),
    remote: inForce({ remote: true, key: true }, { remote: false, started: true, key: true }),
    pending: ['tts.provider'],
  },
  'to-local-voice': {
    ...remote,
    config: asks('local', 'gpt-4o-mini-tts', 'marin'),
    remote: inForce({ remote: true, key: true }, { remote: true, started: true, key: true }),
    pending: ['tts.provider'],
  },
  // A key set and no voice named. The speaker refuses to start here, and no
  // other state reaches that form.
  'no-voice-named': {
    ...remote,
    config: asks('remote', 'gpt-4o-mini-tts', ''),
    remote: inForce({ remote: true, key: true }, { remote: true, started: false, key: true }),
  },
  'no-voice': {
    ...remote,
    config: asks('remote', 'tts-1', ''),
    remote: inForce({ remote: true, key: true }, { remote: true, started: false, key: false }),
  },
  'speech-failed': {
    ...remote,
    config: asks('remote', 'gpt-4o-mini-tts', 'marin'),
    remote: inForce({ remote: true, key: true }, { remote: true, started: true, key: true }),
    last_speech_error: 'the remote speaker refused the key',
  },
  // A key pasted, a voice named, and the speaker still did not start, so no
  // one field is the fix. No other state reaches that form.
  'speaker-not-started': {
    ...remote,
    config: asks('remote', 'gpt-4o-mini-tts', 'marin'),
    remote: inForce({ remote: true, key: true }, { remote: true, started: false, key: true }),
  },
  // The reader left a speaker that never started, and the daemon could not use
  // the choice yet. Both halves of that are true at once here and nowhere else.
  'to-local-not-started': {
    ...remote,
    config: asks('local', 'gpt-4o-mini-tts', 'marin'),
    remote: inForce({ remote: true, key: true }, { remote: true, started: false, key: true }),
    pending: ['tts.provider'],
  },
};

function chosen(): string {
  if (typeof window === 'undefined') return 'ready';
  return new URLSearchParams(window.location.search).get('state') ?? 'ready';
}

// Written here, not captured from anyone. Uneven by intent: tidy one-liners
// hide the wrapping.
const SAID = [
  'Wrap the upload call in a retry with backoff and log each attempt',
  'run the tests and tell me what broke',
  'Why is the second request slower than the first one?',
  'no, revert that last change',
  'Rename the handler so its name says what it gives back, not when it runs',
  'put it behind a flag for now',
  'explain what this regular expression matches',
  'commit that with a message about the timeout',
];

const ASKED = { kind: 'question' as const, question: 'Which branch should I use?' };

function rows(): HistoryRow[] {
  const state = chosen();
  if (state === 'empty' || state === 'first-run' || state === 'downloading') return [];
  // Preview scale only: `?rows=500` repeats the fixture so a long record can
  // be looked at in a browser. Capped, so a typo cannot ask for a million.
  const asked = Number(new URLSearchParams(window.location.search).get('rows') ?? 0);
  const total =
    Number.isFinite(asked) && asked > 0 ? Math.min(Math.floor(asked), 5000) : SAID.length;
  const start = new Date();
  start.setHours(21, 58, 0, 0);
  const at = (i: number) => new Date(start.getTime() - i * 11 * 60_000);
  const week = (i: number) => new Date(at(i).getTime() + 7 * 86_400_000).toISOString();
  // The newest rows show each kind the detail can draw. The preview-scale
  // repeat below stays plain dictations, kept forever, kept a week, or from
  // before audio was kept.
  const kinds: ((i: number) => Omit<HistoryRow, 'id' | 'timestamp'>)[] = [
    (i) => ({
      ...ASKED,
      text: 'The release branch.',
      outcome: 'transcribed',
      reason: null,
      audio: { seconds: 4, until: week(i) },
    }),
    () => ({ ...ASKED, text: null, outcome: 'no_answer', reason: null, audio: null }),
    () => ({ ...ASKED, text: null, outcome: 'closed', reason: null, audio: null }),
    (i) => ({
      ...ASKED,
      text: null,
      outcome: 'failed',
      reason: 'The audio was empty.',
      audio: { seconds: 3, until: week(i) },
    }),
    () => ({
      ...ASKED,
      text: null,
      outcome: 'unasked',
      reason: 'Banshee was not listening.',
      audio: null,
    }),
    (i) => ({
      kind: 'dictation',
      question: null,
      text: null,
      outcome: 'failed',
      reason: 'The transcription server refused the key.',
      audio: { seconds: 11, until: week(i) },
    }),
    () => ({
      kind: 'dictation',
      question: null,
      text: null,
      outcome: 'failed',
      reason: 'The transcription server refused the key.',
      audio: null,
    }),
  ];
  const lead = Math.min(kinds.length, total);
  const kept = [
    () => ({ seconds: 6, until: null }),
    (i: number) => ({ seconds: 2.4, until: week(i) }),
    () => null,
  ];
  // The daemon answers oldest first.
  return Array.from({ length: total }, (_, i) => ({
    id: total - i,
    timestamp: at(i).toISOString(),
    ...(i < lead
      ? kinds[i](i)
      : {
          kind: 'dictation' as const,
          question: null,
          text: SAID[i % SAID.length],
          outcome: 'transcribed' as const,
          reason: null,
          audio: kept[i % kept.length](i),
        }),
  })).reverse();
}

// A soft tone of the row's length, in the daemon's format: 16 kHz mono 16-bit PCM.
function wav(seconds: number): ArrayBuffer {
  const rate = 16_000;
  const samples = Math.round(seconds * rate);
  const buffer = new ArrayBuffer(44 + samples * 2);
  const view = new DataView(buffer);
  const text = (at: number, word: string) =>
    [...word].forEach((c, i) => view.setUint8(at + i, c.charCodeAt(0)));
  text(0, 'RIFF');
  view.setUint32(4, 36 + samples * 2, true);
  text(8, 'WAVE');
  text(12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, 'data');
  view.setUint32(40, samples * 2, true);
  for (let n = 0; n < samples; n += 1) {
    const t = n / rate;
    const fade = Math.min(1, t * 20, (seconds - t) * 20);
    view.setInt16(44 + n * 2, Math.round(Math.sin(2 * Math.PI * 330 * t) * 4000 * fade), true);
  }
  return buffer;
}

function statusNow(): unknown {
  const base = (STATES[chosen()] ?? ready) as Record<string, unknown>;
  if (Object.keys(written).length === 0) return base;
  const config = JSON.parse(JSON.stringify(base.config ?? {})) as Record<
    string,
    Record<string, unknown>
  >;
  for (const [key, value] of Object.entries(written)) {
    const path = key.split('.');
    const leaf = path.pop() as string;
    let node = config as Record<string, unknown>;
    for (const step of path) {
      node[step] = { ...((node[step] as Record<string, unknown>) ?? {}) };
      node = node[step] as Record<string, unknown>;
    }
    node[leaf] = value;
  }
  return { ...base, config };
}

const ANSWERS: Record<string, (args?: Record<string, unknown>) => unknown> = {
  status: statusNow,
  history: () => rows(),
  history_audio: (args) => {
    const audio = rows().find((row) => row.id === args?.id)?.audio;
    if (!audio) throw new Error('No audio is kept for this row.');
    return wav(audio.seconds);
  },
  retry: (args) => {
    const row = rows().find((one) => one.id === args?.id);
    if (!row?.audio) throw new Error('No audio is kept for this row.');
    if (chosen() === 'retry-fails') throw new Error('The model returned no text.');
    return { ...row, text: SAID[0], outcome: 'transcribed', reason: null };
  },
  settings_table: () => settingsJson,
  set_setting: () => [],
  // A failure has no other way to be looked at: every other state is a status
  // reply, and this one only happens when a call rejects.
  open_licence: () => undefined,
  copy_text: () => {
    if (chosen() === 'copy-fails') throw new Error('the clipboard refused it');
    return undefined;
  },
  list_devices: (): Devices => ({
    devices: [
      { name: 'MacBook Pro Microphone', default: true },
      { name: 'OnePlus Buds 3', default: false },
    ],
    current: 'OnePlus Buds 3',
  }),
  list_voices: (): Voices => ({
    voices: [
      {
        id: 'companion',
        name: 'Companion',
        description: 'Pocket TTS by Kyutai, CC-BY-4.0',
        downloaded: true,
      },
      { id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true },
      { id: 'af_bella', name: 'Bella', description: 'American, warm', downloaded: false },
      { id: 'af_heart', name: 'Heart', description: 'American, soft', downloaded: true },
      { id: 'af_nicole', name: 'Nicole', description: 'American, hushed', downloaded: false },
      { id: 'af_sarah', name: 'Sarah', description: 'American, even', downloaded: false },
      { id: 'am_adam', name: 'Adam', description: 'American, low', downloaded: false },
      { id: 'am_michael', name: 'Michael', description: 'American, steady', downloaded: false },
      { id: 'am_santa', name: 'Santa', description: 'American, deep', downloaded: false },
      { id: 'bf_emma', name: 'Emma', description: 'British, bright', downloaded: false },
      { id: 'bf_isabella', name: 'Isabella', description: 'British, warm', downloaded: false },
      { id: 'bm_george', name: 'George', description: 'British, steady', downloaded: false },
      { id: 'bm_lewis', name: 'Lewis', description: 'British, low', downloaded: false },
    ],
    current: 'af_sky',
  }),
  // The home screen's agent absence needs a state with none connected, or it
  // cannot be looked at.
  detect_agents: (): AgentRow[] =>
    chosen() === 'no-agents'
      ? [
          { id: 'claude', name: 'Claude Code', presence: 'found', note: '' },
          { id: 'cursor', name: 'Cursor', presence: 'found', note: '' },
        ]
      : [
          { id: 'claude', name: 'Claude Code', presence: 'connected', note: '' },
          { id: 'codex', name: 'Codex', presence: 'connected', note: '' },
          { id: 'cursor', name: 'Cursor', presence: 'found', note: '' },
          { id: 'opencode', name: 'OpenCode', presence: 'connected', note: '' },
          { id: 'antigravity', name: 'Antigravity', presence: 'absent', note: '' },
          { id: 'pi', name: 'Pi', presence: 'absent', note: '' },
        ],
  plan_connect: (): PlannedChange[] => [
    {
      path: '~/.cursor/mcp.json',
      diff: '+  "banshee": {\n+    "command": "banshee-mcp-shim"\n+  }',
    },
  ],
};

// The daemon pushes nothing into a browser, so this module scripts the one
// stream that has no other way to be seen. Without this the download bar,
// which stands in for the longest wait in the product, is unreviewable.
export function push(event: string, deliver: (payload: unknown) => void): () => void {
  if (event !== 'daemon:downloads' || chosen() !== 'downloading') return () => {};
  // `mb` is the percentage's denominator, from the daemon's real byte count.
  const files = [
    {
      label: 'Listening model',
      model: 'parakeet-unified-en-0.6b-Q8_0.gguf',
      index: 1,
      count: 4,
      mb: 697,
    },
    { label: 'Voice detection', model: 'silero_vad.onnx', index: 2, count: 4, mb: 2 },
  ];
  const at_ = (file: (typeof files)[number], percent: number) => {
    const { mb, ...rest } = file;
    const total = mb * 1_048_576;
    return { ...rest, bytes: Math.round((total * percent) / 100), total, state: 'downloading' };
  };
  let at = 0;
  let done = 0;
  const tick = setInterval(() => {
    done += 7;
    if (done > 100) {
      done = 0;
      at += 1;
      if (at >= files.length) return clearInterval(tick);
    }
    deliver(at_(files[at], done));
  }, 400);
  deliver(at_(files[0], 41));
  return () => clearInterval(tick);
}

export function answer<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (command === 'set_setting' && typeof args?.key === 'string') {
    written[args.key] = args.value;
  }
  const reply = ANSWERS[command];
  try {
    return Promise.resolve((reply ? reply(args) : undefined) as T);
  } catch (refused) {
    return Promise.reject(refused);
  }
}
