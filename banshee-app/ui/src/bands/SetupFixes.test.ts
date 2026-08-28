import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import permissions from '../fixtures/permissions.json';
import { daemon, empty, reduceStatus } from '../lib/daemon';
vi.mock('../lib/tauri', () => ({
  openPermissionPane: vi.fn().mockResolvedValue(null),
  downloadModels: vi.fn().mockResolvedValue(null),
  copyText: vi.fn().mockResolvedValue(null),
}));
import { openPermissionPane } from '../lib/tauri';
import SetupFixes from './SetupFixes.svelte';

beforeEach(() => {
  vi.clearAllMocks();
  daemon.set(reduceStatus(empty(), permissions));
});

it('names the missing grant, its consequence, a pane button and the daemon\'s own fix', () => {
  render(SetupFixes);
  expect(screen.getByRole('button', { name: /Open Accessibility settings/ })).toBeTruthy();
  expect(screen.getByText(/dictation cannot type and the hotkey stays inert/)).toBeTruthy();
  expect(screen.getAllByText(/Turn on Banshee in the list that opens/).length).toBeGreaterThan(0);
  expect(
    screen.getByText('grant it in System Settings > Privacy & Security > Accessibility'),
  ).toBeTruthy();
  expect(screen.getByText(/Try it opens once the rows above are clear/)).toBeTruthy();
});

it('counts the grants the daemon is waiting on', () => {
  render(SetupFixes);
  // The captured machine reports both Accessibility and Input Monitoring.
  expect(screen.getByText(/2 permissions to grant/)).toBeTruthy();
});

it('opens the pane the blocker names, not the one its label reads', async () => {
  render(SetupFixes);
  await fireEvent.click(screen.getByRole('button', { name: /Open Input Monitoring settings/ }));
  expect(vi.mocked(openPermissionPane)).toHaveBeenCalledWith('input_monitoring');
});

it('offers no command for a permission, which names a pane instead', () => {
  render(SetupFixes);
  expect(screen.queryByRole('button', { name: 'Copy command' })).toBeNull();
});

it('puts every missing model under one row, since one call downloads them all', () => {
  daemon.set(
    reduceStatus(empty(), {
      running: true,
      blockers: [
        { kind: 'model', id: 'a.bin', name: 'a.bin', consequence: 'recording does not work', fix: 'run: banshee setup', command: 'banshee setup' },
        { kind: 'model', id: 'b.onnx', name: 'b.onnx', consequence: 'recording does not work', fix: 'run: banshee setup', command: 'banshee setup' },
      ],
    }),
  );
  render(SetupFixes);
  expect(screen.getAllByRole('button', { name: 'Download models' }).length).toBe(1);
  expect(screen.getByText('a.bin, b.onnx')).toBeTruthy();
});

it('says the command once, not as prose and again as a command', () => {
  daemon.set(
    reduceStatus(empty(), {
      running: true,
      blockers: [
        { kind: 'model', id: 'a.bin', name: 'a.bin', consequence: 'recording does not work', fix: 'run: banshee setup', command: 'banshee setup' },
      ],
    }),
  );
  render(SetupFixes);
  expect(screen.queryByText('run: banshee setup')).toBeNull();
  expect(screen.getByText('banshee setup')).toBeTruthy();
});

it('offers the command a missing model names', () => {
  daemon.set(
    reduceStatus(empty(), {
      running: true,
      blockers: [
        {
          kind: 'model',
          id: 'silero_vad.onnx',
          name: 'silero_vad.onnx',
          consequence: 'recording, dictation, and ask_user do not work',
          fix: 'run: banshee setup',
          command: 'banshee setup',
        },
      ],
    }),
  );
  render(SetupFixes);
  expect(screen.getByRole('button', { name: 'Download models' })).toBeTruthy();
  expect(screen.getByText('banshee setup')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Copy command' })).toBeTruthy();
});

it('says "them" when the row stands for more than one thing', () => {
  daemon.set(
    reduceStatus(empty(), {
      running: true,
      blockers: [
        { kind: 'model', id: 'a.bin', name: 'a.bin', consequence: 'recording does not work', fix: 'run: banshee setup', command: 'banshee setup' },
        { kind: 'model', id: 'b.onnx', name: 'b.onnx', consequence: 'recording does not work', fix: 'run: banshee setup', command: 'banshee setup' },
      ],
    }),
  );
  render(SetupFixes);
  expect(screen.getByText(/Without them, recording does not work/)).toBeTruthy();
});

it('offers the one thing a dead daemon can still be told', () => {
  daemon.set({ ...reduceStatus(empty(), permissions), down: 'not running' });
  render(SetupFixes);
  expect(screen.getByText('Banshee is not running.')).toBeTruthy();
  expect(screen.getByText('banshee start')).toBeTruthy();
  // Nothing else can be acted on until it is back.
  expect(screen.queryByRole('button', { name: /Open Accessibility settings/ })).toBeNull();
});

it('offers no button it cannot wire, since the window cannot spawn a daemon', () => {
  daemon.set({ ...reduceStatus(empty(), permissions), down: 'not running' });
  render(SetupFixes);
  expect(screen.queryByRole('button', { name: /^Open |^Download / })).toBeNull();
  expect(screen.getByRole('button', { name: 'Copy command' })).toBeTruthy();
});
