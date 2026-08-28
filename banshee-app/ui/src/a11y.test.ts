import { render } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, expect, it, vi } from 'vitest';
import ready from './fixtures/ready.json';
import permissions from './fixtures/permissions.json';
import recording from './fixtures/recording.json';
import { daemon, empty, reduceLive, reduceStatus } from './lib/daemon';
import { OPENABLE, open } from './lib/jobs';

// One saved dictation is enough for the history panel to render its search
// field and its row controls, which is the ground the gate needs to cover.
const { ONE_ROW } = vi.hoisted(() => ({
  ONE_ROW: [{ id: 1, text: 'Yes.', timestamp: '2026-08-27T11:30:00.000Z' }],
}));

vi.mock('./lib/tauri', () => ({
  status: vi.fn().mockResolvedValue(ready),
  history: vi.fn().mockResolvedValue(ONE_ROW),
  listen: vi.fn().mockResolvedValue(() => {}),
  copyText: vi.fn().mockResolvedValue(undefined),
  listDevices: vi.fn().mockResolvedValue({ devices: [], current: null }),
  listVoices: vi.fn().mockResolvedValue({ voices: [], current: null }),
  detectAgents: vi.fn().mockResolvedValue([]),
  setSetting: vi.fn().mockResolvedValue([]),
  previewVoice: vi.fn().mockResolvedValue(undefined),
  planConnect: vi.fn().mockResolvedValue([]),
  applyConnect: vi.fn().mockResolvedValue(undefined),
  clearHistory: vi.fn().mockResolvedValue(undefined),
}));
import App from './App.svelte';

afterEach(() => open.set(null));

const states = {
  ready: reduceStatus(empty(), ready),
  permissions: reduceStatus(empty(), permissions),
  recording: reduceLive(reduceStatus(empty(), ready), recording),
};

// jsdom has no layout engine, so axe cannot compute colour contrast; the
// palette is verified by hand instead. Left enabled, the rule only makes
// axe probe a canvas jsdom does not implement, and logs a warning for it.
const RUN_OPTIONS = { rules: { 'color-contrast': { enabled: false } } };

for (const [name, state] of Object.entries(states)) {
  it(`has no axe violations in the ${name} state`, async () => {
    daemon.set(state);
    const { container } = render(App);
    const results = await axe.run(container, RUN_OPTIONS);
    expect(results.violations).toEqual([]);
  });
}

// A job panel only renders while `$open` names it, so the three states above
// never reach inside one. Each openable panel gets its own run.
for (const name of OPENABLE) {
  it(`has no axe violations with the ${name} panel open`, async () => {
    daemon.set(states.ready);
    open.set(name);
    const { container } = render(App);
    const results = await axe.run(container, RUN_OPTIONS);
    expect(results.violations).toEqual([]);
  });
}
