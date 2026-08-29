import { invoke } from '@tauri-apps/api/core';
import type { Status } from './daemon';

export { listen } from '@tauri-apps/api/event';

export type CommandError = { code: number; message: string };
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
  return invoke('status');
}
export function setSetting(key: string, value: unknown): Promise<string[]> {
  return invoke('set_setting', { key, value });
}
export function listDevices(): Promise<Devices> {
  return invoke('list_devices');
}
export function listVoices(): Promise<Voices> {
  return invoke('list_voices');
}
export function previewVoice(id: string): Promise<void> {
  return invoke('preview_voice', { id });
}
export function downloadModels(): Promise<void> {
  return invoke('download_models');
}
export function detectAgents(): Promise<AgentRow[]> {
  return invoke('detect_agents');
}
export function planConnect(id: string, disconnect: boolean): Promise<PlannedChange[]> {
  return invoke('plan_connect', { id, disconnect });
}
export function applyConnect(id: string, disconnect: boolean): Promise<void> {
  return invoke('apply_connect', { id, disconnect });
}
export function history(limit?: number): Promise<HistoryRow[]> {
  return invoke('history', { limit: limit ?? null });
}
export function clearHistory(): Promise<void> {
  return invoke('clear_history');
}
export function openPermissionPane(id: string): Promise<void> {
  return invoke('open_permission_pane', { id });
}
export function copyText(text: string): Promise<void> {
  return invoke('copy_text', { text });
}

export function startDaemon(): Promise<void> {
  return invoke('start_daemon');
}
