import { render, screen } from '@testing-library/svelte';
import { expect, it, vi } from 'vitest';
vi.mock('../lib/tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));
import Pad from './Pad.svelte';
it('shows the latest dictation at 17px with a Copy button', () => {
  render(Pad, { latest: { text: 'Yes, open the pull request.', timestamp: '2026-08-26T13:47:00Z' }, landing: null, agent: null });
  expect(screen.getByText('Yes, open the pull request.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Copy' })).toBeTruthy();
});
it('names the agent that asks', () => {
  render(Pad, { latest: null, landing: null, agent: { who: 'Claude Code', text: 'Open the pull request?' } });
  expect(screen.getByText('Claude Code asks')).toBeTruthy();
});
it('shows landing words with no Copy button', () => {
  render(Pad, { latest: null, landing: 'Add a test for', agent: null });
  expect(screen.queryByRole('button', { name: 'Copy' })).toBeNull();
});
it('offers Stop speaking only while the daemon speaks', async () => {
  const { daemon } = await import('../lib/daemon');
  const agent = { who: 'Claude Code', text: 'Open the pull request?' };
  render(Pad, { latest: null, landing: null, agent });
  expect(screen.queryByRole('button', { name: 'Stop speaking' })).toBeNull();
  daemon.update((s) => ({ ...s, live: { ...s.live, speaking: true } }));
  expect(await screen.findByRole('button', { name: 'Stop speaking' })).toBeTruthy();
  daemon.update((s) => ({ ...s, live: { ...s.live, speaking: false } }));
});
