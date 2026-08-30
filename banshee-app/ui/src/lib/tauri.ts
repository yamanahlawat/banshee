import { invoke } from '@tauri-apps/api/core';
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event';
import type { Status } from './daemon';

// `import.meta.env.DEV` is replaced with `false` in a production build, so the
// preview branch and its module are dropped rather than merely unreachable.
const PREVIEW =
  import.meta.env.DEV &&
  !import.meta.env.VITEST &&
  !(typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window);

function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (PREVIEW) return import('./preview').then((m) => m.answer<T>(command, args));
  return invoke<T>(command, args);
}

// The daemon pushes nothing into a browser, so a preview listens to silence
// rather than throwing on a bridge that is not there.
export function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  if (PREVIEW) return Promise.resolve(() => {});
  return tauriListen<T>(event, handler);
}

export type InputDevice = { name: string; default: boolean };
export type Devices = { devices: InputDevice[]; current: string | null };
export type Voice = { id: string; name: string; description: string };
export type Voices = { voices: Voice[]; current: string | null };
export type AgentRow = { id: string; name: string; presence: string; note: string };
export type PlannedChange = { path: string | null; diff: string };
export type DownloadProgress = { state: 'downloading' | 'done' | 'failed' };
export type Down = { reason: string };
export type HistoryRow = { id: string | number; text: string; timestamp: string };

export function status(): Promise<Status> {
  return call('status');
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
export function applyConnect(id: string, disconnect: boolean): Promise<void> {
  return call('apply_connect', { id, disconnect });
}
export function history(limit?: number): Promise<HistoryRow[]> {
  return call('history', { limit: limit ?? null });
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

export function startDaemon(): Promise<void> {
  return call('start_daemon');
}
