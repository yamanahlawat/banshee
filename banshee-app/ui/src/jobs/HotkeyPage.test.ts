import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { setSetting } from '../lib/tauri';
import { daemon, empty, type Status } from '../lib/daemon';
import { table } from '../lib/settings';
import { commonTable, tableWith } from '../lib/tauri.mock';
import HotkeyPage from './HotkeyPage.svelte';

// The set the parser reports on Linux, from bansheed/src/binding.rs.
const ON_LINUX = ['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightControl'];

function ready(bindable: string[] = ON_LINUX) {
  const audio: Record<string, unknown> = {
    hotkey: 'RightOption',
    hotkey_mode: 'hold',
    barge_in: 'stop',
  };
  const config: Record<string, unknown> = { audio };
  const status = {
    running: true,
    hotkey_listens: true,
    bindable_modifiers: bindable,
    config,
  } as unknown as Status;
  daemon.update((state) => ({ ...state, status, pending: new Set<string>() }));
}

async function capture(code: string, held: Partial<KeyboardEventInit> = {}) {
  const page = render(HotkeyPage);
  await fireEvent.click(page.getByRole('button', { name: /change the hotkey/ }));
  await waitFor(() => expect(page.getByText('Press a key')).toBeTruthy());
  await fireEvent.keyDown(window, { code, ...held });
  await fireEvent.keyUp(window, { code, ...held });
  return page;
}

beforeEach(() => {
  table.set(commonTable);
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
  const page = await capture('MetaRight');
  await waitFor(() => expect(page.getByText('Banshee cannot bind that key.')).toBeTruthy());
  expect(setSetting).not.toHaveBeenCalled();
});

it('turns the same two around when the daemon reports the other platform', async () => {
  ready(['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightCommand', 'Fn']);
  await capture('MetaRight');
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('audio.hotkey', 'RightCommand'));
});

it('binds an F-key on the press, without waiting for a release', async () => {
  const page = render(HotkeyPage);
  await fireEvent.click(page.getByRole('button', { name: /change the hotkey/ }));
  await fireEvent.keyDown(window, { code: 'F6' });
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('audio.hotkey', 'F6'));
});

it('refuses every Shift form, which the daemon ignores', async () => {
  const page = await capture('KeyR', { shiftKey: true });
  await waitFor(() => expect(page.getByText('Banshee cannot bind that key.')).toBeTruthy());
  expect(setSetting).not.toHaveBeenCalled();
});

it('offers the compositor commands instead of a capture on Wayland', () => {
  daemon.update((state) => ({
    ...state,
    status: { ...state.status, hotkey_listens: false } as unknown as Status,
  }));
  const page = render(HotkeyPage);

  expect(page.queryByRole('button', { name: /change the hotkey/ })).toBeNull();
  expect(page.getByText(/^banshee record start banshee record stop$/)).toBeTruthy();
});

// With no daemon the window is told no modifier binds, which is not the same
// as a key Banshee refuses.
it('says what is really wrong when no daemon has answered', async () => {
  daemon.set(empty());
  const page = render(HotkeyPage);
  await fireEvent.click(page.getByRole('button', { name: /change the hotkey/ }));
  await fireEvent.keyDown(window, { code: 'ControlLeft' });

  expect(page.getByText(/has to be running/)).toBeTruthy();
  expect(page.queryByText(/cannot bind that key/)).toBeNull();
});

// The window is opened most often when Banshee has stopped. Hiding the key and
// naming a compositor leaves a macOS user with no way to see their own hotkey.
it('still offers the key when no daemon has answered', () => {
  daemon.set(empty());
  const page = render(HotkeyPage);
  expect(page.getByRole('button', { name: /change the hotkey/ })).toBeTruthy();
  expect(page.queryByText(/Wayland grants no global hotkey/)).toBeNull();
});

it('leaves Feedback to its own page', () => {
  const page = render(HotkeyPage);
  expect(page.queryByRole('radiogroup', { name: 'Feedback' })).toBeNull();
});

it('offers the press behaviours in the order the table lists them', () => {
  table.set(
    tableWith('audio.hotkey_mode', { domain: { kind: 'choice', values: ['toggle', 'hold'] } }),
  );
  const { getByRole } = render(HotkeyPage);
  const modes = getByRole('radiogroup', { name: 'Press behaviour' });
  expect(
    [...modes.querySelectorAll('[role="radio"]')].map((radio) => radio.textContent?.trim()),
  ).toEqual(['Tap', 'Hold']);
});
