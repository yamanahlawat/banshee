import { invoke } from '@tauri-apps/api/core';
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event';
import type { Status } from './daemon';

// `import.meta.env.DEV` is replaced with `false` in a production build, so the
// branch and its module are dropped rather than merely unreachable.
const USE_MOCKS =
  import.meta.env.DEV &&
  !import.meta.env.VITEST &&
  !(typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window);

function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (USE_MOCKS) return import('../mocks/bridge').then((m) => m.answer<T>(command, args));
  return invoke<T>(command, args);
}

// The daemon pushes nothing into a browser, so the mocks listen to silence
// rather than throw on a bridge that is not there.
export function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  if (USE_MOCKS) {
    return import('../mocks/bridge').then((m) =>
      m.push(event, (payload) => handler({ payload } as Parameters<EventCallback<T>>[0])),
    );
  }
  return tauriListen<T>(event, handler);
}

type InputDevice = { name: string; default: boolean };
export type Devices = { devices: InputDevice[]; current: string | null };
export type Voice = {
  id: string;
  name: string;
  description: string;
  downloaded: boolean;
};
export type Voices = { voices: Voice[]; current: string | null };
export type AgentRow = { id: string; name: string; presence: string; note: string };
export type PlannedChange = { path: string | null; diff: string };
/// The `daemon:downloads` payload. One shape, declared beside the helpers that
/// read it: `label`, `index` and `count` are absent from a daemon older than
/// them, and `total` is null when the server sent no Content-Length.
export type { Progress as DownloadProgress } from './daemon';
export type Down = { reason: string };
export type Reopened = { missed: boolean };
export type HistoryRow = {
  id: string | number;
  kind: 'dictation' | 'question';
  question: string | null;
  text: string | null;
  outcome: 'transcribed' | 'failed' | 'no_answer' | 'closed' | 'unasked';
  reason: string | null;
  timestamp: string;
  /// The program that asked, on a question row only.
  client: string | null;
  /// `until` is local RFC 3339, and null while Keep audio is Forever.
  audio: { seconds: number; until: string | null } | null;
};

export type Domain =
  | { kind: 'toggle' }
  | { kind: 'choice'; values: string[] }
  | { kind: 'number'; min: number; max: number | null; whole: boolean }
  | { kind: 'words' }
  | { kind: 'text'; empty_allowed: boolean }
  | { kind: 'url' }
  | { kind: 'hotkey' }
  | { kind: 'listening_model' }
  | { kind: 'voice' }
  | { kind: 'secret' };
/// A row of the table in `banshee-common`. `default` is the JSON text of what an absent key reads as.
export type Setting = {
  key: string;
  domain: Domain;
  default: string | null;
  sends_out: boolean;
  description: string;
};

export function status(): Promise<Status> {
  return call('status');
}
export function settingsTable(): Promise<Setting[]> {
  return call('settings_table');
}
export function setSetting(key: string, value: unknown): Promise<string[]> {
  return call('set_setting', { key, value });
}
export function listDevices(): Promise<Devices> {
  return call('list_devices');
}
export function listVoices(): Promise<Voices> {
  return call('list_voices');
}
export function previewVoice(id: string): Promise<void> {
  return call('preview_voice', { id });
}
export function downloadModels(): Promise<void> {
  return call('download_models');
}
export function detectAgents(): Promise<AgentRow[]> {
  return call('detect_agents');
}
export function planConnect(id: string, disconnect: boolean): Promise<PlannedChange[]> {
  return call('plan_connect', { id, disconnect });
}
export function applyConnect(id: string, disconnect: boolean): Promise<string | null> {
  return call('apply_connect', { id, disconnect });
}
export function history(limit?: number): Promise<HistoryRow[]> {
  return call('history', { limit: limit ?? null });
}
export function historyAudio(id: HistoryRow['id']): Promise<ArrayBuffer> {
  return call('history_audio', { id });
}
/// Transcribes the row's kept audio again, and answers the row as it now stands.
export function retry(id: HistoryRow['id']): Promise<HistoryRow> {
  return call('retry', { id });
}
export function clearHistory(): Promise<void> {
  return call('clear_history');
}
export function openPermissionPane(id: string): Promise<void> {
  return call('open_permission_pane', { id });
}
export function copyText(text: string): Promise<void> {
  return call('copy_text', { text });
}
/// Opens the licence of a listening model, named by its id.
export function openLicence(model: string): Promise<void> {
  return call('open_licence', { model });
}

/// Whether the tray started this process for a missed question, true once.
export function launchMissed(): Promise<boolean> {
  return call('launch_missed');
}

export function startDaemon(): Promise<void> {
  return call('start_daemon');
}
/// Replaces the running daemon, which a pipeline that died at startup needs.
/// `startDaemon` is for a daemon that is down.
export function restartDaemon(): Promise<void> {
  return call('restart_daemon');
}
