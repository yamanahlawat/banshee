import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { beforeEach, expect, it, vi } from 'vitest';
import ready from './mocks/ready.json';
import permissions from './mocks/permissions.json';
import { daemon, empty, type Blocker } from './lib/daemon';

// `axe.run` builds its tree synchronously from the DOM it is handed, and every
// onMount here suspends at its first await, so each run below first waits for
// content that only appears once that state's own fetch has landed.

const ROWS = vi.hoisted(() => {
  const now = new Date();
  const at = (hour: number) =>
    new Date(now.getFullYear(), now.getMonth(), now.getDate(), hour, 0, 0).toISOString();
  const row = { reason: null, question: null };
  return [
    {
      id: 3,
      ...row,
      kind: 'question',
      question: 'Which branch should I use?',
      text: 'The release branch.',
      outcome: 'transcribed',
      timestamp: at(11),
    },
    {
      id: 2,
      ...row,
      kind: 'dictation',
      text: 'Open the pull request.',
      outcome: 'transcribed',
      timestamp: at(9),
    },
    {
      id: 1,
      ...row,
      kind: 'dictation',
      text: 'Yes.',
      outcome: 'transcribed',
      timestamp: at(10),
    },
  ];
});

vi.mock('./lib/tauri', async () => (await import('./lib/tauri.mock')).mockTauri());

import { detectAgents, history, listen, listDevices, listVoices, status } from './lib/tauri';
import type { HistoryRow } from './lib/tauri';
import { agents } from './lib/agents';
import { table as historyTable, view } from './lib/history';
import { forgetKeys } from './lib/keys';
import App from './App.svelte';

import { PAGES } from './lib/nav';

const choose = (name: string) =>
  fireEvent.click(
    within(screen.getByRole('navigation', { name: 'Banshee' })).getByRole('button', {
      name: new RegExp(`^${name}`),
    }),
  );

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
  view.set({ query: '', chosen: null });
  forgetKeys();
  vi.mocked(status).mockResolvedValue(ready);
  vi.mocked(history).mockResolvedValue(ROWS as HistoryRow[]);
  vi.mocked(listen).mockResolvedValue(() => {});
  vi.mocked(listDevices).mockResolvedValue({
    devices: [{ name: 'MacBook Pro Microphone', default: true }],
    current: 'MacBook Pro Microphone',
  });
  vi.mocked(listVoices).mockResolvedValue({
    voices: [{ id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true }],
    current: 'af_sky',
  });
  vi.mocked(detectAgents).mockResolvedValue([
    { id: 'claude', name: 'Claude Code', presence: 'found', note: '' },
  ]);
});

it('the conversation carries no accessibility violations', async () => {
  const { container } = render(App);
  await screen.findByRole('option', { name: /^Yes\./ });
  expect(await violations(container)).toEqual([]);
});

// The window opens on h2s, so a reader jumping by headings starts mid-rank.
it('names itself once at the top of the heading order', async () => {
  render(App);
  await screen.findByRole('option', { name: /^Yes\./ });
  expect(screen.getByRole('heading', { level: 1, name: 'Banshee' })).toBeTruthy();
});

it('a blocked machine carries none either', async () => {
  vi.mocked(status).mockResolvedValue({
    ...permissions,
    blockers: permissions.blockers as Blocker[],
  });
  const { container } = render(App);
  await screen.findAllByRole('button', { name: /^Open System Settings for / });
  expect(await violations(container)).toEqual([]);
});

it.each(PAGES)('the %s page carries none', async (name) => {
  const { container } = render(App);
  await screen.findByRole('option', { name: /^Yes\./ });
  await choose(name);
  await waitFor(() => expect(screen.getByRole('region', { name })).toBeTruthy());
  expect(await violations(container)).toEqual([]);
});

// axe fails a group role on the list itself, so the group wraps a plain one.
it('holds the settings in a group that wraps a plain list', async () => {
  render(App);
  await screen.findByRole('option', { name: /^Yes\./ });
  const settings = screen.getByRole('group', { name: 'Settings' });
  expect(settings.tagName).toBe('DIV');
  const list = settings.querySelector(':scope > ul');
  expect(list?.hasAttribute('role')).toBe(false);
});

// This page holds the only action in the window that cannot be undone, so the
// confirm that replaces its controls is checked as well as the page.
it('the Saved history page carries none during the confirm', async () => {
  const { container } = render(App);
  await screen.findByRole('option', { name: /^Yes\./ });

  await choose('Saved history');
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Keep it' })).toBeTruthy());
  expect(await violations(container)).toEqual([]);
});
