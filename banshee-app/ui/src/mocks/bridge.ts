// Reached only under `import.meta.env.DEV` and only when Tauri is absent, so
// it cannot ship.
import ready from './ready.json';
import permissions from './permissions.json';
import notRunning from './not-running.json';
import remote from './remote.json';
import remoteSpeech from './remote-speech.json';
import type { HistoryRow } from '../lib/tauri';

// A write changes what `status` answers next, as the daemon's would.
const written: Record<string, unknown> = {};

const HOST = 'api.openai.com';
const OPENAI = `https://${HOST}/v1`;

// What the daemon runs, beside a config that says only what was asked for. A
// side the daemon runs remotely always reaches this one host.
function inForce(
  listener: { remote: boolean; key: boolean },
  speaker: { remote: boolean; started: boolean; key: boolean },
) {
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
// window shows neither `response_format` nor `sample_rate`, so this names them
// no value to look at.
function asks(provider: string, model: string, voice: string) {
  return {
    ...remote.config,
    tts: {
      ...remote.config.tts,
      provider,
      remote: { base_url: OPENAI, model, voice, instructions: '' },
    },
  };
}

const STATES: Record<string, unknown> = {
  ready,
  permissions,
  remote,
  'remote-speech': remoteSpeech,
  'not-running': notRunning,
  'no-agents': ready,
  'copy-fails': ready,
  // A real first run: blocked, and nothing ever said.
  'first-run': permissions,
  downloading: permissions,
  recording: { ...ready, recording: true },
  speaking: { ...ready, speaking: true },
  armed: { ...ready, recording: true, armed: true },
  saving_off: { ...ready, config: { ...ready.config, daemon: { save_history: false } } },
  pending: { ...ready, pending: ['audio.hotkey', 'daemon.save_history'] },
  // The listener the config asks for and the one the daemon runs, in each
  // direction. Neither has any other way to be looked at: the pair only
  // disagrees between a write and the restart that applies it.
  'to-remote': {
    ...remote,
    remote: inForce({ remote: false, key: true }, { remote: false, started: true, key: false }),
    pending: ['stt.provider'],
  },
  'to-local': {
    ...remote,
    config: { ...remote.config, stt: { ...remote.config.stt, provider: 'local' } },
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
  // The reader left a speaker that never started, and the flip waits on the
  // restart. Both halves of that are true at once here and nowhere else.
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

function rows(): HistoryRow[] {
  const state = chosen();
  if (
    state === 'empty' ||
    state === 'saving_off' ||
    state === 'first-run' ||
    state === 'downloading'
  )
    return [];
  const start = new Date();
  start.setHours(21, 58, 0, 0);
  // The daemon answers oldest first.
  return SAID.map((text, i) => ({
    id: SAID.length - i,
    text,
    timestamp: new Date(start.getTime() - i * 11 * 60_000).toISOString(),
  })).reverse();
}

function statusNow(): unknown {
  const base = (STATES[chosen()] ?? ready) as Record<string, unknown>;
  if (Object.keys(written).length === 0) return base;
  const config = JSON.parse(JSON.stringify(base.config ?? {})) as Record<
    string,
    Record<string, unknown>
  >;
  for (const [key, value] of Object.entries(written)) {
    // `audio.cues.enabled` is three segments deep, not two.
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

const ANSWERS: Record<string, () => unknown> = {
  status: statusNow,
  history: () => (written['daemon.save_history'] === false ? [] : rows()),
  set_setting: () => [],
  // A failure has no other way to be looked at: every other state is a status
  // reply, and this one only happens when a call rejects.
  copy_text: () => {
    if (chosen() === 'copy-fails') throw new Error('the clipboard refused it');
    return undefined;
  },
  list_devices: () => ({
    devices: [
      { name: 'MacBook Pro Microphone', default: true },
      { name: 'OnePlus Buds 3', default: false },
    ],
    current: 'OnePlus Buds 3',
  }),
  // Whisper's own order: English first, the rest by how much training data each
  // had. A short slice of it, because a mock needs a list and not the list.
  list_languages: () => ({
    languages: [
      { code: 'en', name: 'English' },
      { code: 'zh', name: 'Chinese' },
      { code: 'de', name: 'German' },
      { code: 'es', name: 'Spanish' },
      { code: 'ru', name: 'Russian' },
      { code: 'ko', name: 'Korean' },
      { code: 'fr', name: 'French' },
      { code: 'ja', name: 'Japanese' },
      { code: 'hi', name: 'Hindi' },
    ],
  }),
  list_voices: () => ({
    voices: [
      { id: 'af_sky', name: 'Sky', description: 'American, clear' },
      { id: 'af_heart', name: 'Heart', description: 'American, warm' },
      { id: 'am_adam', name: 'Adam', description: 'American, low' },
    ],
    current: 'af_sky',
  }),
  // The home screen's agent absence needs a state with none connected, or it
  // cannot be looked at.
  detect_agents: () =>
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
  plan_connect: () => [
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
  const files = [
    { label: 'Speech model', model: 'ggml-large-v3-turbo.bin', index: 1, count: 4 },
    { label: 'Voice detection', model: 'silero_vad.onnx', index: 2, count: 4 },
  ];
  let at = 0;
  let done = 0;
  const tick = setInterval(() => {
    done += 7;
    if (done > 100) {
      done = 0;
      at += 1;
      if (at >= files.length) return clearInterval(tick);
    }
    deliver({ ...files[at], bytes: done, total: 100, state: 'downloading' });
  }, 400);
  deliver({ ...files[0], bytes: 41, total: 100, state: 'downloading' });
  return () => clearInterval(tick);
}

export function answer<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (command === 'set_setting' && typeof args?.key === 'string') {
    written[args.key] = args.value;
  }
  const reply = ANSWERS[command];
  try {
    return Promise.resolve((reply ? reply() : undefined) as T);
  } catch (refused) {
    return Promise.reject(refused);
  }
}
