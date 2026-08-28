import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

const { detectAgents, planConnect, applyConnect } = vi.hoisted(() => ({
  detectAgents: vi.fn(),
  planConnect: vi.fn(),
  applyConnect: vi.fn(),
}));
vi.mock('../lib/tauri', () => ({ detectAgents, planConnect, applyConnect }));
import { showCommands } from '../lib/settings';
import Agents from './Agents.svelte';

beforeEach(() => {
  vi.clearAllMocks();
  showCommands.set(false);
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

it('shows a connected agent by name and note, with no button to undo it', async () => {
  detectAgents.mockResolvedValue([{ id: 'claude-code', name: 'Claude Code', presence: 'connected', note: 'Connected' }]);
  render(Agents);
  await screen.findByText('Connected');
  expect(screen.queryByRole('button')).toBeNull();
});

it("renders the daemon's refusal as a sentence on the row rather than opening a review", async () => {
  const refusal = 'Installed, but the plan failed: no write access.';
  planConnect.mockRejectedValue({ message: refusal });
  render(Agents);
  await fireEvent.click(await screen.findByRole('button', { name: 'Connect' }));
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
  expect(screen.queryByRole('button')).toBeNull();
});

it('prints the command that connects it once Show commands is on', async () => {
  showCommands.set(true);
  render(Agents);
  expect(await screen.findByText('banshee connect cursor')).toBeTruthy();
});

it('prints no command for an agent that is already connected or not installed', async () => {
  showCommands.set(true);
  detectAgents.mockResolvedValue([
    { id: 'claude-code', name: 'Claude Code', presence: 'connected', note: 'Connected' },
    { id: 'pi', name: 'Pi', presence: 'absent', note: 'Not installed. Looked for pi on PATH' },
  ]);
  render(Agents);
  await screen.findByText('Connected');
  expect(screen.queryByText(/^banshee connect/)).toBeNull();
});

it('prints nothing while Show commands is off', async () => {
  render(Agents);
  await screen.findByText('Found');
  expect(screen.queryByText('banshee connect cursor')).toBeNull();
});
