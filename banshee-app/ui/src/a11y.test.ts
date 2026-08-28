import { render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ready from './fixtures/ready.json';
import permissionsFixture from './fixtures/permissions.json';
import recordingLive from './fixtures/recording.json';
import { daemon, empty } from './lib/daemon';
import { OPENABLE, open } from './lib/jobs';

// `axe.run` builds its tree synchronously from the DOM it is handed, and
// every onMount here suspends at its first await. So each run below first
// awaits a query that stays false until that state's own fetch has landed.

// `today()` filters by local day, so both rows anchor to a fixed hour of
// now's own day rather than to an elapsed offset. Two rows leave OLDER a
// seat in the band whether or not the pad claims one.
const { OLDER, NEWER } = vi.hoisted(() => {
  const now = new Date();
  const localHour = (hour: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate(), hour, 0, 0).toISOString();
  return {
    OLDER: { id: 2, text: 'Open the pull request.', timestamp: localHour(9) },
    NEWER: { id: 1, text: 'Yes.', timestamp: localHour(10) },
  };
});

const {
  status,
  history,
  listen,
  copyText,
  listDevices,
  listVoices,
  detectAgents,
  setSetting,
  previewVoice,
  planConnect,
  applyConnect,
  clearHistory,
  downloadModels,
  openPermissionPane,
} = vi.hoisted(() => ({
  status: vi.fn(),
  history: vi.fn(),
  listen: vi.fn().mockResolvedValue(() => {}),
  copyText: vi.fn().mockResolvedValue(undefined),
  listDevices: vi.fn(),
  listVoices: vi.fn(),
  detectAgents: vi.fn(),
  setSetting: vi.fn().mockResolvedValue([]),
  previewVoice: vi.fn().mockResolvedValue(undefined),
  planConnect: vi.fn().mockResolvedValue([]),
  applyConnect: vi.fn().mockResolvedValue(undefined),
  clearHistory: vi.fn().mockResolvedValue(undefined),
  downloadModels: vi.fn().mockResolvedValue(undefined),
  openPermissionPane: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('./lib/tauri', () => ({
  status,
  history,
  listen,
  copyText,
  listDevices,
  listVoices,
  detectAgents,
  setSetting,
  previewVoice,
  planConnect,
  applyConnect,
  clearHistory,
  downloadModels,
  openPermissionPane,
}));
import App from './App.svelte';

beforeEach(() => {
  daemon.set(empty());
  open.set(null);
  status.mockReset().mockResolvedValue(ready);
  history.mockReset().mockResolvedValue([OLDER, NEWER]);
  listDevices.mockReset().mockResolvedValue({ devices: [], current: null });
  listVoices.mockReset().mockResolvedValue({ voices: [], current: null });
  detectAgents.mockReset().mockResolvedValue([]);
});

afterEach(() => open.set(null));

// jsdom has no layout engine, so axe cannot compute colour contrast; the
// palette is verified by hand instead. Left enabled, the rule only makes
// axe probe a canvas jsdom does not implement, and logs a warning for it.
const RUN_OPTIONS = { rules: { 'color-contrast': { enabled: false } } };

it('has no axe violations in the ready state', async () => {
  const { container } = render(App);
  await screen.findByText(OLDER.text);
  expect((await axe.run(container, RUN_OPTIONS)).violations).toEqual([]);
});

it('has no axe violations in the permissions state', async () => {
  status.mockResolvedValue(permissionsFixture);
  const { container } = render(App);
  await screen.findByText(OLDER.text);
  expect((await axe.run(container, RUN_OPTIONS)).violations).toEqual([]);
});

it('has no axe violations in the recording state', async () => {
  // The live flags travel on the same status reply the app already reads,
  // so merging the recorded fixtures reaches the recording state through
  // the real reducer path.
  status.mockResolvedValue({ ...ready, ...recordingLive });
  const { container } = render(App);
  await screen.findByText(OLDER.text);
  expect((await axe.run(container, RUN_OPTIONS)).violations).toEqual([]);
});

// A panel renders only while `$open` names it, so each proof waits on data
// that panel alone can hold. A control that exists in the empty state
// proves nothing.
const PANEL_PROOF: Record<string, () => Promise<unknown>> = {
  Microphone: () => screen.findByRole('option', { name: 'MacBook Pro Microphone' }),
  // The strip always shows the same humanized value in its own row, so the
  // job's copy of it is not the only match once data has landed.
  Hotkey: () => screen.findAllByText('Right Command'),
  Voice: () => screen.findAllByText('Sky'),
  Agents: () => screen.findByText('Cursor'),
  'More settings': () => screen.findByText(OLDER.text),
};

for (const name of OPENABLE) {
  it(`has no axe violations with the ${name} panel open`, async () => {
    if (name === 'Microphone') {
      listDevices.mockResolvedValue({ devices: [{ name: 'MacBook Pro Microphone', default: true }], current: 'MacBook Pro Microphone' });
    }
    if (name === 'Voice') {
      listVoices.mockResolvedValue({ voices: [{ id: 'af_sky', name: 'Sky', description: 'American, clear' }], current: 'af_sky' });
    }
    if (name === 'Agents') {
      detectAgents.mockResolvedValue([{ id: 'cursor', name: 'Cursor', presence: 'found', note: 'Found' }]);
    }
    open.set(name);
    const { container } = render(App);
    await PANEL_PROOF[name]();
    expect((await axe.run(container, RUN_OPTIONS)).violations).toEqual([]);
  });
}
