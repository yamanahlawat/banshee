import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

const { detectAgents, planConnect, applyConnect } = vi.hoisted(() => ({
  detectAgents: vi.fn(),
  planConnect: vi.fn(),
  applyConnect: vi.fn(),
}));
vi.mock('../lib/tauri', () => ({ detectAgents, planConnect, applyConnect }));
import Agents from './Agents.svelte';

beforeEach(() => {
  vi.clearAllMocks();
  applyConnect.mockResolvedValue(null);
  detectAgents.mockResolvedValue([{ id: 'cursor', name: 'Cursor', presence: 'found', note: 'Found' }]);
  planConnect.mockResolvedValue([{ path: '~/.cursor/mcp.json', diff: '+ "banshee": {' }]);
});

it('shows the diff and writes nothing until Apply', async () => {
  render(Agents);
  await fireEvent.click(await screen.findByRole('button', { name: 'Connect' }));
  expect(await screen.findByText(/Nothing is written until you apply it/)).toBeTruthy();
  expect(screen.getByText('Reviewing')).toBeTruthy();
  expect(applyConnect).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
  expect(applyConnect).toHaveBeenCalledWith('cursor', false);
});

it('writes nothing and closes the review when Not now is pressed', async () => {
  render(Agents);
  await fireEvent.click(await screen.findByRole('button', { name: 'Connect' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Not now' }));
  expect(applyConnect).not.toHaveBeenCalled();
  expect(screen.queryByText('Reviewing')).toBeNull();
  expect(await screen.findByRole('button', { name: 'Connect' })).toBeTruthy();
});

it('shows Disconnect for a connected agent and plans with disconnect true', async () => {
  detectAgents.mockResolvedValue([{ id: 'claude-code', name: 'Claude Code', presence: 'connected', note: 'Connected' }]);
  planConnect.mockRejectedValue({ message: "Disconnect is not available yet. Remove Banshee from the agent's config by hand." });
  render(Agents);
  await fireEvent.click(await screen.findByRole('button', { name: 'Disconnect' }));
  expect(planConnect).toHaveBeenCalledWith('claude-code', true);
});

it("renders the daemon's refusal as a sentence on the row rather than opening a review", async () => {
  detectAgents.mockResolvedValue([{ id: 'claude-code', name: 'Claude Code', presence: 'connected', note: 'Connected' }]);
  const refusal = "Disconnect is not available yet. Remove Banshee from the agent's config by hand.";
  planConnect.mockRejectedValue({ message: refusal });
  render(Agents);
  await fireEvent.click(await screen.findByRole('button', { name: 'Disconnect' }));
  expect(await screen.findByText(refusal)).toBeTruthy();
  expect(screen.queryByText('Reviewing')).toBeNull();
  expect(applyConnect).not.toHaveBeenCalled();
});

it("shows the daemon's refusal and closes the review when Apply itself fails", async () => {
  planConnect.mockResolvedValue([{ path: '~/.cursor/mcp.json', diff: '+ "banshee": {' }]);
  applyConnect.mockRejectedValue({ message: 'The file changed since the plan was made.' });
  render(Agents);
  await fireEvent.click(await screen.findByRole('button', { name: 'Connect' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
  expect(await screen.findByText('The file changed since the plan was made.')).toBeTruthy();
  expect(screen.queryByText('Reviewing')).toBeNull();
});

it('does not open a review when the plan has no changes to show', async () => {
  planConnect.mockResolvedValue([]);
  render(Agents);
  await fireEvent.click(await screen.findByRole('button', { name: 'Connect' }));
  expect(screen.queryByText('Reviewing')).toBeNull();
  expect(await screen.findByRole('button', { name: 'Connect' })).toBeTruthy();
  expect(applyConnect).not.toHaveBeenCalled();
});

it('offers no action for an agent that is not installed', async () => {
  detectAgents.mockResolvedValue([{ id: 'pi', name: 'Pi', presence: 'absent', note: 'Not installed. Looked for pi on PATH' }]);
  render(Agents);
  await screen.findByText('Not installed. Looked for pi on PATH');
  expect(screen.queryByRole('button', { name: 'Connect' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Disconnect' })).toBeNull();
});
