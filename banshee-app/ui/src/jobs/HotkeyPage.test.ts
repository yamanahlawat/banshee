import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { setSetting } from '../lib/tauri';
import { daemon, empty, type Status } from '../lib/daemon';
import { forgetRefusals, table } from '../lib/settings';
import { commonTable, tableWith } from '../lib/tauri.mock';
import { expectInOrder } from '../lib/order.mock';
import HotkeyPage from './HotkeyPage.svelte';

// The set the parser reports on Linux, from bansheed/src/binding.rs.
const ON_LINUX = ['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightControl'];

function ready(bindable: string[] = ON_LINUX, feedback?: Record<string, unknown>) {
  const audio: Record<string, unknown> = {
    hotkey: 'RightOption',
    hotkey_mode: 'hold',
    barge_in: 'stop',
  };
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
  const page = render(HotkeyPage);
  await fireEvent.click(page.getByRole('button', { name: /change the hotkey/ }));
  await waitFor(() => expect(page.getByText('Press a key')).toBeTruthy());
  await fireEvent.keyDown(window, { code, ...held });
  await fireEvent.keyUp(window, { code, ...held });
  return page;
}

beforeEach(() => {
  table.set(commonTable);
  forgetRefusals();
  vi.mocked(setSetting).mockReset();
  vi.mocked(setSetting).mockResolvedValue(undefined as never);
  daemon.set(empty());
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
  const page = await capture('MetaRight');
  await waitFor(() => expect(page.getByText('Banshee cannot bind that key.')).toBeTruthy());
  expect(setSetting).not.toHaveBeenCalled();
});

it('says a key it cannot bind on the key row', async () => {
  const page = await capture('MetaRight');
  const refusal = await waitFor(() => page.getByText('Banshee cannot bind that key.'));
  const key = page.getByRole('button', { name: /change the hotkey/ });
  expect(refusal.closest('.row')?.contains(key)).toBe(true);
  expect(key.getAttribute('aria-describedby')).toBe(refusal.id);
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

// A stopped Banshee sends no config, so the key is unknown rather than unset.
it('says the key is unknown while Banshee is stopped, and shows it once Banshee runs', async () => {
  const page = render(HotkeyPage);
  daemon.update((state) => ({ ...state, down: 'not running' }));
  const key = page.getByRole('button', { name: /change the hotkey/ });
  await waitFor(() =>
    expect(key.textContent).toBe('Unknown while Banshee is stopped, change the hotkey'),
  );

  daemon.update((state) => ({ ...state, down: null }));
  await waitFor(() => expect(key.textContent).toBe('Right Option, change the hotkey'));
});

it('still says the key is unset when a running Banshee binds none', () => {
  daemon.update((state) => ({
    ...state,
    status: { ...state.status, config: { audio: {} } } as Status,
  }));
  const page = render(HotkeyPage);
  expect(page.getByRole('button', { name: /change the hotkey/ }).textContent).toBe(
    'Not set, change the hotkey',
  );
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

it('holds one button for Change and Cancel, ghost while it waits', async () => {
  const page = render(HotkeyPage);
  const change = page.getByRole('button', { name: 'Change' });
  expect(change.classList.contains('btn-ghost')).toBe(false);
  expect(change.querySelector('.held')?.textContent).toBe('Change');

  await fireEvent.click(change);
  const cancel = page.getByRole('button', { name: 'Cancel' });
  expect(cancel).toBe(change);
  expect(cancel.classList.contains('btn-ghost')).toBe(true);
  expect(cancel.querySelector('.held')?.textContent).toBe('Change');
});

it('marks the capture as waiting only while it waits for a key', async () => {
  const page = render(HotkeyPage);
  const capture = page.getByRole('button', { name: /change the hotkey/ });
  expect(capture.classList.contains('waiting')).toBe(false);

  await fireEvent.click(capture);
  expect(capture.classList.contains('waiting')).toBe(true);
});

it('says what Stop and Carry on do', () => {
  const page = render(HotkeyPage);
  const group = page.getByRole('radiogroup', { name: 'While Banshee is talking' });
  const note = document.getElementById(group.getAttribute('aria-describedby') ?? '');
  expect(note?.textContent).toBe(
    'Stop: Banshee goes quiet when you press the key. Carry on: it finishes what it is saying.',
  );
});

it('words Stop and Carry on for dictation on Wayland', () => {
  daemon.update((state) => ({
    ...state,
    status: { ...state.status, hotkey_listens: false } as unknown as Status,
  }));
  const page = render(HotkeyPage);
  const group = page.getByRole('radiogroup', { name: 'While Banshee is talking' });
  const note = document.getElementById(group.getAttribute('aria-describedby') ?? '');
  expect(note?.textContent).toBe(
    'Stop: Banshee goes quiet when you start dictating. Carry on: it finishes what it is saying.',
  );
});

const MAC_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
const LINUX_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15';

// A daemon set to visual differs by platform, so each test names its own user
// agent rather than the one jsdom happens to report. `inForce` is what the
// daemon reports, which a config still asking for something else does not move.
function withFeedback(
  options: { userAgent?: string; feedback?: Record<string, unknown>; inForce?: string } = {},
) {
  if (options.userAgent !== undefined) {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(options.userAgent);
  }
  ready(ON_LINUX, options.feedback);
  if (options.inForce !== undefined) {
    const feedback = options.inForce;
    daemon.update((state) => ({ ...state, live: { ...state.live, feedback } }));
  }
  return render(HotkeyPage);
}

function feedbackNote(page: Pick<ReturnType<typeof withFeedback>, 'getByRole'>): string {
  const group = page.getByRole('radiogroup', { name: 'Feedback' });
  return document.getElementById(group.getAttribute('aria-describedby') ?? '')?.textContent ?? '';
}

it('holds Feedback last, after While Banshee is talking', () => {
  const page = withFeedback();
  expectInOrder(
    page.getByRole('radiogroup', { name: 'While Banshee is talking' }),
    page.getByRole('radiogroup', { name: 'Feedback' }),
  );
});

it('keeps Feedback where the compositor holds the key', () => {
  daemon.update((state) => ({
    ...state,
    status: { ...state.status, hotkey_listens: false } as unknown as Status,
  }));
  const page = render(HotkeyPage);
  expect(page.getByRole('radiogroup', { name: 'Feedback' })).toBeTruthy();
});

it('points the feedback radiogroup at its note', () => {
  const page = withFeedback({ feedback: { mode: 'sound' } });
  const group = page.getByRole('radiogroup', { name: 'Feedback' });
  const note = page.getByText(/A failure plays a sound too\.$/);
  expect(group.getAttribute('aria-describedby')).toBe(note.id);
});

// The daemon says which cues are in force; the config says only what was asked.
it('opens the feedback note with the cues the daemon reports', () => {
  const page = withFeedback({ userAgent: MAC_UA, feedback: { mode: 'both' }, inForce: 'none' });
  expect(feedbackNote(page)).toBe(
    'Banshee shows nothing and plays no sound. A failure plays a sound too.',
  );
});

it('names the place the figure shows, per platform', () => {
  const mac = withFeedback({ userAgent: MAC_UA, feedback: { mode: 'both' }, inForce: 'both' });
  expect(feedbackNote(mac)).toBe(
    'Banshee shows the figure above the Dock and plays a sound when it starts and stops listening. A failure plays a sound too.',
  );
  mac.unmount();
  vi.restoreAllMocks();

  const linux = withFeedback({ userAgent: LINUX_UA, feedback: { mode: 'both' }, inForce: 'both' });
  expect(feedbackNote(linux)).toBe(
    'Banshee shows the figure at the bottom of the screen and plays a sound when it starts and stops listening. A failure plays a sound too.',
  );
});

it('says only that a stopped Banshee shows and plays nothing, and the mode note returns once it runs', async () => {
  const page = withFeedback({ userAgent: MAC_UA, feedback: { mode: 'both' }, inForce: 'both' });
  daemon.update((state) => ({ ...state, down: 'not running' }));
  await waitFor(() =>
    expect(feedbackNote(page)).toBe('Banshee is not running, so it shows and plays nothing.'),
  );

  daemon.update((state) => ({ ...state, down: null }));
  await waitFor(() => expect(feedbackNote(page)).toMatch(/A failure plays a sound too\.$/));
});

it('offers On screen on macOS and writes visual', async () => {
  const page = withFeedback({ userAgent: MAC_UA });
  await fireEvent.click(page.getByRole('radio', { name: 'On screen' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('feedback.mode', 'visual'));
});

it('selects On screen for a daemon set to visual, with the VoiceOver advice', () => {
  const page = withFeedback({ userAgent: MAC_UA, feedback: { mode: 'visual' } });
  const chosen = page.getByRole('radio', { name: 'On screen' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
  expect(feedbackNote(page)).toBe(
    "An agent's questions are still spoken aloud. Using VoiceOver? Choose Both, so you hear when recording starts.",
  );
});

it('offers On screen on Linux, selects it for visual, and names the layer shell', () => {
  const page = withFeedback({ userAgent: LINUX_UA, feedback: { mode: 'visual' } });
  const chosen = page.getByRole('radio', { name: 'On screen' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
  expect(feedbackNote(page)).toBe(
    "An agent's questions are still spoken aloud. The figure needs a desktop with layer shell, such as Hyprland, Sway or KDE. Without one, Banshee plays every sound.",
  );
});

// The sentence before it says what shows and what plays, so the rest says only
// what that sentence leaves out.
it('notes the failure sound under Both, and the spoken questions under Off', () => {
  const both = withFeedback({ userAgent: LINUX_UA, feedback: { mode: 'both' } });
  expect(feedbackNote(both)).toBe('A failure plays a sound too.');
  both.unmount();

  const off = withFeedback({ userAgent: MAC_UA, feedback: { mode: 'none' } });
  expect(feedbackNote(off)).toBe("An agent's questions are still spoken aloud.");
});

it('writes feedback.mode', async () => {
  const page = withFeedback();
  await fireEvent.click(page.getByRole('radio', { name: 'Sound' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('feedback.mode', 'sound'));
});
