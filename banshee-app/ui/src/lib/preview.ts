// Stands in for the Tauri bridge in an ordinary browser. Reached only under
// `import.meta.env.DEV` and only when Tauri is absent, so it cannot ship.
// `?state=` picks which daemon reply to answer with.
import ready from '../fixtures/ready.json';
import permissions from '../fixtures/permissions.json';
import notRunning from '../fixtures/not-running.json';
import type { HistoryRow } from './tauri';

// A write changes what `status` answers next, as the daemon's would.
const written: Record<string, unknown> = {};

const STATES: Record<string, unknown> = {
  ready,
  permissions,
  'not-running': notRunning,
  recording: { ...ready, recording: true },
  speaking: { ...ready, speaking: true },
  armed: { ...ready, recording: true, armed: true },
  saving_off: { ...ready, config: { ...ready.config, daemon: { save_history: false } } },
};

function chosen(): string {
  if (typeof window === 'undefined') return 'ready';
  return new URLSearchParams(window.location.search).get('state') ?? 'ready';
}

// Written for the preview, not captured from anyone. Uneven by intent: tidy
// one-liners hide the wrapping.
const SAID = [
  'you can do that, it will be a redesign of everything',
  "what's pending from the task or can i start with ui design",
  "Did we run post code checks for the previous commits we did? If not, let's do that including the current changes and run the comments feedback guidelines and then we will be ready to commit",
  "Let's do it.",
  'So what do we do about it? Should that only work when user restarts banshee?',
  'if stte preset and details fallback cannot be live at all should we just remove it from the UI',
  'commit the changes and push the branch',
  'read the surface brief before you touch anything',
];

function rows(): HistoryRow[] {
  const state = chosen();
  if (state === 'empty' || state === 'saving_off') return [];
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
  list_devices: () => ({
    devices: [
      { name: 'MacBook Pro Microphone', default: true },
      { name: 'OnePlus Buds 3', default: false },
    ],
    current: 'OnePlus Buds 3',
  }),
  list_voices: () => ({
    voices: [
      { id: 'af_sky', name: 'Sky', description: 'American, clear' },
      { id: 'af_heart', name: 'Heart', description: 'American, warm' },
      { id: 'am_adam', name: 'Adam', description: 'American, low' },
    ],
    current: 'af_sky',
  }),
  detect_agents: () => [
    { id: 'claude', name: 'Claude Code', presence: 'connected', note: '' },
    { id: 'codex', name: 'Codex', presence: 'connected', note: '' },
    { id: 'cursor', name: 'Cursor', presence: 'found', note: '' },
    { id: 'opencode', name: 'OpenCode', presence: 'connected', note: '' },
    { id: 'antigravity', name: 'Antigravity', presence: 'absent', note: '' },
    { id: 'pi', name: 'Pi', presence: 'absent', note: '' },
  ],
  plan_connect: () => [
    { path: '~/.cursor/mcp.json', diff: '+  "banshee": {\n+    "command": "banshee-mcp-shim"\n+  }' },
  ],
};

export function answer<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (command === 'set_setting' && typeof args?.key === 'string') {
    written[args.key] = args.value;
  }
  const reply = ANSWERS[command];
  return Promise.resolve((reply ? reply() : undefined) as T);
}
