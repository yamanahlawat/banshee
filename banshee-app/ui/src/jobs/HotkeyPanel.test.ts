import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { setSetting } from '../lib/tauri';
import { daemon, type Status } from '../lib/daemon';
import HotkeyPanel from './HotkeyPanel.svelte';

// The set the parser reports on Linux, from bansheed/src/binding.rs.
const ON_LINUX = ['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightControl'];

function ready(bindable: string[] = ON_LINUX) {
  const status = {
    running: true,
    hotkey_listens: true,
    bindable_modifiers: bindable,
    config: { audio: { hotkey: 'RightOption', hotkey_mode: 'hold', barge_in: 'stop' } },
  } as unknown as Status;
  daemon.update((state) => ({ ...state, status, pending: new Set<string>() }));
}

async function capture(code: string, held: Partial<KeyboardEventInit> = {}) {
  const panel = render(HotkeyPanel);
  await fireEvent.click(panel.getByRole('button', { name: /change the hotkey/ }));
  await waitFor(() => expect(panel.getByText('Press a key')).toBeTruthy());
  await fireEvent.keyDown(window, { code, ...held });
  await fireEvent.keyUp(window, { code, ...held });
  return panel;
}

beforeEach(() => {
  vi.mocked(setSetting).mockReset();
  vi.mocked(setSetting).mockResolvedValue(undefined as never);
  ready();
});

// The daemon reports which modifiers it binds, and the set differs per
// platform. A table of the window's own drifts from it.
it('binds a modifier the daemon reports', async () => {
  await capture('ControlRight');
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('audio.hotkey', 'RightControl'));
});

it('refuses a modifier the daemon does not report, and says so', async () => {
  const panel = await capture('MetaRight');
  await waitFor(() => expect(panel.getByText('Banshee cannot bind that key.')).toBeTruthy());
  expect(setSetting).not.toHaveBeenCalled();
});

it('turns the same two around when the daemon reports the other platform', async () => {
  ready(['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightCommand', 'Fn']);
  await capture('MetaRight');
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('audio.hotkey', 'RightCommand'));
});

it('binds an F-key on the press, without waiting for a release', async () => {
  const panel = render(HotkeyPanel);
  await fireEvent.click(panel.getByRole('button', { name: /change the hotkey/ }));
  await fireEvent.keyDown(window, { code: 'F6' });
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('audio.hotkey', 'F6'));
});

it('refuses every Shift form, which the daemon reserves', async () => {
  const panel = await capture('KeyR', { shiftKey: true });
  await waitFor(() => expect(panel.getByText('Banshee cannot bind that key.')).toBeTruthy());
  expect(setSetting).not.toHaveBeenCalled();
});

it('offers the compositor commands instead of a capture on Wayland', () => {
  daemon.update((state) => ({
    ...state,
    status: { ...state.status, hotkey_listens: false } as unknown as Status,
  }));
  const panel = render(HotkeyPanel);

  expect(panel.queryByRole('button', { name: /change the hotkey/ })).toBeNull();
  expect(panel.getByText(/banshee record start --dictate/)).toBeTruthy();
});
