import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { applyConnect, detectAgents, planConnect, type AgentRow } from '../lib/tauri';
import { agents } from '../lib/agents';
import AgentsPanel from './AgentsPanel.svelte';

const CLAUDE: AgentRow = {
  id: 'claude',
  name: 'Claude Code',
  presence: 'found',
  note: 'Installed, not connected',
};

beforeEach(() => {
  agents.set([]);
  vi.mocked(detectAgents).mockReset();
  vi.mocked(planConnect).mockReset();
  vi.mocked(applyConnect).mockReset();
});

// A read that failed is not a machine with no agents. Left saying it is still
// looking, the panel reports a green check the daemon never gave, and this
// audience has no terminal to fall back to.
it('says the list could not be read, and offers to read it again', async () => {
  vi.mocked(detectAgents).mockRejectedValueOnce(new Error('no daemon'));
  const { getByRole, getByText, queryByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText(/could not read which agents/i)).toBeTruthy());
  expect(queryByText(/Looking for agents/)).toBeNull();

  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  await fireEvent.click(getByRole('button', { name: /Look again/ }));

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  expect(queryByText(/could not read which agents/i)).toBeNull();
});

// An empty list after a read that landed is a real answer, and a different one.
it('says the machine has no agents once a read has landed', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([]);
  const { getByText, queryByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText(/No coding agent is installed/i)).toBeTruthy());
  expect(queryByText(/Looking for agents/)).toBeNull();
});

// The write landed, so the panel may not report a failure. Only the list it
// draws is doubtful, and it says which of the two happened.
it('states that the agent connected when the list cannot be read afterwards', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockResolvedValue(undefined);
  const { getByRole, getByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Apply' })).toBeTruthy());

  vi.mocked(detectAgents).mockRejectedValueOnce(new Error('no daemon'));
  await fireEvent.click(getByRole('button', { name: 'Apply' }));

  await waitFor(() => expect(getByText(/Claude Code is connected/)).toBeTruthy());
  expect(getByText(/may be out of date/)).toBeTruthy();
});
