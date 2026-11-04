import { vi } from 'vitest';
import mockTable from '../mocks/settings.json';
// Type only: a value import of './tauri' would load the module this file mocks.
import type { Setting } from './tauri';

/// The table the common crate holds, as the panels read it.
export const commonTable = mockTable as Setting[];

/// The common table with one row changed.
export function tableWith(key: string, patch: Partial<Setting>): Setting[] {
  return commonTable.map((row) => (row.key === key ? { ...row, ...patch } : row));
}

/// Every command `lib/tauri.ts` exposes, mocked. Both suites call this so a new
/// command is added in one place rather than in two identical lists.
export function mockTauri() {
  return {
    status: vi.fn(),
    history: vi.fn(),
    listen: vi.fn(),
    copyText: vi.fn(() => Promise.resolve()),
    clearHistory: vi.fn(() => Promise.resolve()),
    settingsTable: vi.fn(() => Promise.resolve(mockTable)),
    setSetting: vi.fn(() => Promise.resolve([])),
    listDevices: vi.fn(),
    previewVoice: vi.fn(() => Promise.resolve()),
    downloadModels: vi.fn(() => Promise.resolve()),
    openPermissionPane: vi.fn(() => Promise.resolve()),
    openLicence: vi.fn(() => Promise.resolve()),
    listVoices: vi.fn(),
    detectAgents: vi.fn(),
    startDaemon: vi.fn(() => Promise.resolve()),
    restartDaemon: vi.fn(() => Promise.resolve()),
    planConnect: vi.fn(),
    applyConnect: vi.fn(),
  };
}
