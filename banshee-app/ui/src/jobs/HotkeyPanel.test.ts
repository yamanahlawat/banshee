import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { setSetting } from '../lib/tauri';
import { daemon, empty, type Status } from '../lib/daemon';
import HotkeyPanel from './HotkeyPanel.svelte';

// The set the parser reports on Linux, from bansheed/src/binding.rs.
const ON_LINUX = ['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightControl'];

const MAC_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
const LINUX_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15';

function ready(
  bindable: string[] = ON_LINUX,
  feedback?: Record<string, unknown>,
  cues?: Record<string, unknown>,
) {
  const audio: Record<string, unknown> = {
    hotkey: 'RightOption',
    hotkey_mode: 'hold',
    barge_in: 'stop',
  };
  if (cues) audio.cues = cues;
  const config: Record<string, unknown> = { audio };
  if (feedback) config.feedback = feedback;
  const status = {
    running: true,
    hotkey_listens: true,
    bindable_modifiers: bindable,
    config,
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

// A daemon set to visual differs by platform, so the panel needs its own user
// agent per test rather than the one jsdom happens to report.
function renderPanel(options: { userAgent?: string; feedback?: Record<string, unknown> } = {}) {
  if (options.userAgent !== undefined) {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(options.userAgent);
  }
  ready(ON_LINUX, options.feedback);
  return render(HotkeyPanel);
}

beforeEach(() => {
  vi.mocked(setSetting).mockReset();
  vi.mocked(setSetting).mockResolvedValue(undefined as never);
  ready();
});

afterEach(() => {
  vi.restoreAllMocks();
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

// With no daemon the window is told no modifier binds, which is not the same
// as a key Banshee refuses.
it('says what is really wrong when no daemon has answered', async () => {
  daemon.set(empty());
  const panel = render(HotkeyPanel);
  await fireEvent.click(panel.getByRole('button', { name: /change the hotkey/ }));
  await fireEvent.keyDown(window, { code: 'ControlLeft' });

  expect(panel.getByText(/has to be running/)).toBeTruthy();
  expect(panel.queryByText(/cannot bind that key/)).toBeNull();
});

// The window is opened most often when Banshee has stopped. Hiding the key and
// naming a compositor leaves a macOS user with no way to see their own hotkey.
it('still offers the key when no daemon has answered', () => {
  daemon.set(empty());
  const panel = render(HotkeyPanel);
  expect(panel.getByRole('button', { name: /change the hotkey/ })).toBeTruthy();
  expect(panel.queryByText(/Wayland grants no global hotkey/)).toBeNull();
});

it('maps a daemon reporting visual to Both, and says it was set from the command line', () => {
  ready(ON_LINUX, { mode: 'visual' });
  const panel = render(HotkeyPanel);
  const chosen = panel.getByRole('radio', { name: 'Both' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
  expect(
    panel.getByText(
      'Set to visual outside this window. Without the on-screen figure, Banshee plays every sound.',
    ),
  ).toBeTruthy();
});

it('points the feedback radiogroup at its note', () => {
  ready(ON_LINUX, { mode: 'sound' });
  const panel = render(HotkeyPanel);
  const group = panel.getByRole('radiogroup', { name: 'Feedback' });
  const note = panel.getByText(
    'A short sound when Banshee starts and stops listening, and when it fails.',
  );
  expect(group.getAttribute('aria-describedby')).toBe(note.id);
});

it('offers On screen on macOS and writes visual', async () => {
  const panel = renderPanel({ userAgent: MAC_UA });
  await fireEvent.click(panel.getByRole('radio', { name: 'On screen' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('feedback.mode', 'visual'));
});

it('selects On screen for a daemon set to visual, with no sounds and the VoiceOver advice', () => {
  const panel = renderPanel({ userAgent: MAC_UA, feedback: { mode: 'visual' } });
  const chosen = panel.getByRole('radio', { name: 'On screen' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
  expect(panel.getByText(/shows what Banshee does, with no sounds/)).toBeTruthy();
  expect(panel.getByText(/An agent's questions are still spoken aloud/)).toBeTruthy();
  expect(panel.getByText(/Using VoiceOver\? Choose Both/)).toBeTruthy();
});

it('offers no On screen where no figure draws', () => {
  const panel = renderPanel({ userAgent: LINUX_UA, feedback: { mode: 'visual' } });
  expect(panel.queryByRole('radio', { name: 'On screen' })).toBeNull();
  const chosen = panel.getByRole('radio', { name: 'Both' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
});

it('names no figure in the Both and Off notes on Linux', () => {
  const both = renderPanel({ userAgent: LINUX_UA, feedback: { mode: 'both' } });
  expect(both.getByText('Every sound.')).toBeTruthy();
  both.unmount();

  const off = renderPanel({ userAgent: LINUX_UA, feedback: { mode: 'none' } });
  expect(off.getByText("No sound. An agent's questions are still spoken aloud.")).toBeTruthy();
});

it('writes feedback.mode, never the retired cue switch', async () => {
  const panel = render(HotkeyPanel);
  await fireEvent.click(panel.getByRole('radio', { name: 'Sound' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('feedback.mode', 'sound'));
  expect(setSetting).not.toHaveBeenCalledWith('audio.cues.enabled', expect.anything());
});

it('reads Off from the old cue switch when an older daemon reports no feedback table', () => {
  ready(ON_LINUX, undefined, { enabled: false });
  const panel = render(HotkeyPanel);
  const chosen = panel.getByRole('radio', { name: 'Off' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
});
