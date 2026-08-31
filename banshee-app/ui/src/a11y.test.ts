import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { beforeEach, expect, it, vi } from 'vitest';
import ready from './fixtures/ready.json';
import permissions from './fixtures/permissions.json';
import { daemon, empty } from './lib/daemon';

// `axe.run` builds its tree synchronously from the DOM it is handed, and every
// onMount here suspends at its first await, so each run below first waits for
// content that only appears once that state's own fetch has landed.

const ROWS = vi.hoisted(() => {
  const now = new Date();
  const at = (hour: number) =>
    new Date(now.getFullYear(), now.getMonth(), now.getDate(), hour, 0, 0).toISOString();
  return [
    { id: 2, text: 'Open the pull request.', timestamp: at(9) },
    { id: 1, text: 'Yes.', timestamp: at(10) },
  ];
});

vi.mock('./lib/tauri', async () => (await import('./lib/tauri.mock')).mockTauri());

import {
  detectAgents,
  history,
  listen,
  listDevices,
  listLanguages,
  listVoices,
  status,
} from './lib/tauri';
import { agents } from './lib/agents';
import { table as historyTable } from './lib/history';
import { forgetKeys } from './lib/keys';
import App from './App.svelte';

const JOBS = ['Microphone', 'Hotkey', 'Voice', 'Agents'];

async function violations(container: HTMLElement): Promise<string[]> {
  const results = await axe.run(container, {
    // No layout engine here, so this rule reaches only an unimplemented canvas
    // and reports "incomplete" while printing a warning per test. The real
    // contrast figures are measured and recorded in the surface brief.
    rules: { 'color-contrast': { enabled: false } },
  });
  return results.violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`);
}

beforeEach(async () => {
  await new Promise((resolve) => setTimeout(resolve, 0));
  vi.clearAllMocks();
  daemon.set(empty());
  agents.set([]);
  historyTable.set({ rows: [], total: 0, loaded: false, saving: null });
  forgetKeys();
  vi.mocked(status).mockResolvedValue(ready);
  vi.mocked(history).mockResolvedValue(ROWS);
  vi.mocked(listen).mockResolvedValue(() => {});
  vi.mocked(listDevices).mockResolvedValue({
    devices: [{ name: 'MacBook Pro Microphone', default: true }],
    current: 'MacBook Pro Microphone',
  });
  vi.mocked(listLanguages).mockResolvedValue({
    languages: [
      { code: 'en', name: 'English' },
      { code: 'hi', name: 'Hindi' },
    ],
  });
  vi.mocked(listVoices).mockResolvedValue({
    voices: [{ id: 'af_sky', name: 'Sky', description: 'American, clear' }],
    current: 'af_sky',
  });
  vi.mocked(detectAgents).mockResolvedValue([
    { id: 'claude', name: 'Claude Code', presence: 'found', note: '' },
  ]);
});

it('the conversation carries no accessibility violations', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes.')).toBeTruthy());
  expect(await violations(container)).toEqual([]);
});

it('a blocked machine carries none either', async () => {
  vi.mocked(status).mockResolvedValue(permissions);
  const { container } = render(App);
  await screen.findAllByRole('button', { name: /^Open System Settings for / });
  expect(await violations(container)).toEqual([]);
});

it.each(JOBS)('the %s panel carries none', async (job) => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes.')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: new RegExp(job) }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toBeTruthy());
  expect(await violations(container)).toEqual([]);
});

// This panel holds the only action in the window that cannot be undone, so the
// confirm that replaces its controls is checked as well as the panel.
it('the Record panel carries none, before and during the confirm', async () => {
  const { container } = render(App);
  await waitFor(() => expect(screen.getByText('Yes.')).toBeTruthy());

  await fireEvent.click(screen.getByRole('button', { name: /saved/ }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toBeTruthy());
  expect(await violations(container)).toEqual([]);

  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Keep it' })).toBeTruthy());
  expect(await violations(container)).toEqual([]);
});
