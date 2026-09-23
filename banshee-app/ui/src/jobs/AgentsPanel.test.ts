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
  vi.mocked(applyConnect).mockResolvedValue(null);
  const { getByRole, getByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Apply' })).toBeTruthy());

  vi.mocked(detectAgents).mockRejectedValueOnce(new Error('no daemon'));
  await fireEvent.click(getByRole('button', { name: 'Apply' }));

  await waitFor(() => expect(getByText(/Claude Code is connected/)).toBeTruthy());
  expect(getByText(/may be out of date/)).toBeTruthy();
});

// Each of the three below destroys the control that was just pressed. axe
// cannot see a lost focus, so it is asserted here.
const CONNECTED: AgentRow = { ...CLAUDE, presence: 'connected', note: '' };

it('moves focus into the review when the plan opens', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  const { getByRole, getByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));

  await waitFor(() => expect(document.activeElement).toBe(getByRole('button', { name: 'Apply' })));
});

it('puts focus back on the connect button the cancel came from', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  const { getByRole, getByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Cancel' })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Cancel' }));

  await waitFor(() =>
    expect(document.activeElement).toBe(getByRole('button', { name: 'Connect' })),
  );
});

// The agent is connected now, so its Connect button is gone.
it('moves focus onto the row an apply just changed', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockResolvedValue(null);
  const { getByRole, getByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Apply' })).toBeTruthy());

  vi.mocked(detectAgents).mockResolvedValueOnce([CONNECTED]);
  await fireEvent.click(getByRole('button', { name: 'Apply' }));

  await waitFor(() => expect(getByText('Connected')).toBeTruthy());
  // `body` holds the whole panel, so its text matches anything: name the node.
  const landed = document.activeElement as HTMLElement;
  expect(landed).not.toBe(document.body);
  expect(landed.classList.contains('agent')).toBe(true);
  expect(landed.textContent).toMatch(/Claude Code/);
});

const CODEX: AgentRow = {
  id: 'codex',
  name: 'Codex',
  presence: 'found',
  note: 'Installed, not connected',
};
const TRUST = 'Codex runs this hook only after you trust it: open Codex and run /hooks.';

// Codex skips a hook nobody trusted. A window user has no terminal line that says so.
it('shows what is left to do after a connect that needs a step', async () => {
  vi.mocked(detectAgents)
    .mockResolvedValueOnce([CODEX])
    .mockResolvedValueOnce([{ ...CODEX, presence: 'connected', note: 'Connected' }]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.codex/hooks.json', diff: '+ turn-end' }]);
  vi.mocked(applyConnect).mockResolvedValue(TRUST);
  const { getByRole, getByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));

  await waitFor(() => expect(getByText(TRUST)).toBeTruthy());
});

it('shows no step after a connect that failed', async () => {
  vi.mocked(detectAgents).mockResolvedValue([CODEX]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.codex/hooks.json', diff: '+ turn-end' }]);
  vi.mocked(applyConnect).mockRejectedValue(new Error('config.toml changed after the plan'));
  const { getByRole, getByText, queryByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));

  await waitFor(() => expect(getByText(/changed after the plan/)).toBeTruthy());
  expect(queryByText(TRUST)).toBeNull();
});

it('drops the step when a later connect fails', async () => {
  vi.mocked(detectAgents)
    .mockResolvedValueOnce([CODEX])
    .mockRejectedValueOnce(new Error('no daemon'));
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.codex/hooks.json', diff: '+ turn-end' }]);
  vi.mocked(applyConnect)
    .mockResolvedValueOnce(TRUST)
    .mockRejectedValueOnce(new Error('config.toml changed after the plan'));
  const { getByRole, getByText, queryByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));
  await waitFor(() => expect(getByText(TRUST)).toBeTruthy());

  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));

  await waitFor(() => expect(getByText(/changed after the plan/)).toBeTruthy());
  expect(queryByText(TRUST)).toBeNull();
});

// A long diff scrolls inside its own box, and a keyboard user has no wheel.
it('lets the keyboard reach each change in the review by its name', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CODEX]);
  vi.mocked(planConnect).mockResolvedValue([
    { path: '~/.codex/hooks.json', diff: '+ turn-end' },
    { path: null, diff: '$ codex mcp add banshee' },
  ]);
  const { getByRole, getByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());

  const file = getByRole('region', { name: 'Changes to ~/.codex/hooks.json' });
  const command = getByRole('region', { name: 'Command to run' });
  expect(file.textContent).toBe('+ turn-end');
  expect(file.tabIndex).toBe(0);
  expect(command.tabIndex).toBe(0);
});

// The header repeats the path shown above the box, and `--- /dev/null` is the
// only other sign of a new file.
it("marks a new file beside its path and drops both diff's headers", async () => {
  const created = {
    path: '~/.codex/hooks.json',
    diff: '--- /dev/null\n+++ b/.codex/hooks.json\n@@ -0,0 +1,2 @@\n+line one\n+line two\n',
  };
  const changed = {
    path: '~/.codex/config.toml',
    diff: '--- a/.codex/config.toml\n+++ b/.codex/config.toml\n@@ -1,2 +1,2 @@\n-old\n+new\n',
  };
  vi.mocked(detectAgents).mockResolvedValueOnce([CODEX]);
  vi.mocked(planConnect).mockResolvedValue([created, changed]);
  const { getByRole, getByText, queryByText } = render(AgentsPanel);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());

  expect(getByText('~/.codex/hooks.json (new file)')).toBeTruthy();
  expect(getByText('~/.codex/config.toml')).toBeTruthy();
  expect(queryByText('~/.codex/config.toml (new file)')).toBeNull();

  const file = getByRole('region', { name: 'New file ~/.codex/hooks.json' });
  const other = getByRole('region', { name: 'Changes to ~/.codex/config.toml' });
  expect(file.textContent).not.toContain('/dev/null');
  expect(file.textContent).not.toContain('+++');
  expect(other.textContent).not.toContain('---');
  expect(other.textContent).not.toContain('+++');
});
