import { vi } from 'vitest';

/// Every command `lib/tauri.ts` exposes, mocked. Both suites call this so a new
/// command is added in one place rather than in two identical lists.
export function mockTauri() {
  return {
    status: vi.fn(),
    history: vi.fn(),
    listen: vi.fn(),
    copyText: vi.fn(() => Promise.resolve()),
    clearHistory: vi.fn(() => Promise.resolve()),
    setSetting: vi.fn(() => Promise.resolve([])),
    listDevices: vi.fn(),
    previewVoice: vi.fn(() => Promise.resolve()),
    downloadModels: vi.fn(() => Promise.resolve()),
    openPermissionPane: vi.fn(() => Promise.resolve()),
    listVoices: vi.fn(),
    detectAgents: vi.fn(),
    startDaemon: vi.fn(() => Promise.resolve()),
    planConnect: vi.fn(),
    applyConnect: vi.fn(),
  };
}
