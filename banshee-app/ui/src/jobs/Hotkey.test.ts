import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('../lib/tauri', () => ({ setSetting: vi.fn(), status: vi.fn(), copyText: vi.fn() }));
import { setSetting, status } from '../lib/tauri';
import ready from '../fixtures/ready.json';
import { daemon, empty, reduceStatus } from '../lib/daemon';
import Hotkey from './Hotkey.svelte';

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue(ready);
  daemon.set(reduceStatus(empty(), ready));
});

it('records the next key only after Change key is pressed, and Escape cancels', async () => {
  render(Hotkey);
  expect(screen.getByText('Right Command')).toBeTruthy();

  // Before the button, a press belongs to whatever else is listening.
  await fireEvent.keyDown(window, { code: 'F6', key: 'F6' });
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();

  await fireEvent.click(screen.getByRole('button', { name: 'Change key' }));
  expect(screen.getByText(/Recording, press a key. Escape cancels./)).toBeTruthy();

  await fireEvent.keyDown(window, { code: 'Escape', key: 'Escape' });
  expect(screen.getByText('Right Command')).toBeTruthy();
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
});

it('sends the key it recorded in the daemon\'s own grammar', async () => {
  render(Hotkey);
  await fireEvent.click(screen.getByRole('button', { name: 'Change key' }));
  await fireEvent.keyDown(window, { code: 'F6', key: 'F6' });
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('audio.hotkey', 'F6');
});

it('says so rather than sending a key the daemon cannot bind', async () => {
  render(Hotkey);
  await fireEvent.click(screen.getByRole('button', { name: 'Change key' }));
  await fireEvent.keyDown(window, { code: 'ShiftLeft', key: 'Shift' });
  expect(screen.getByRole('alert').textContent).toContain('cannot bind');
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
});

it('shows the daemon\'s own refusal rather than a guess at one', async () => {
  vi.mocked(setSetting).mockRejectedValue({ message: 'Shift is reserved for capitals.' });
  render(Hotkey);
  await fireEvent.click(screen.getByRole('button', { name: 'Change key' }));
  await fireEvent.keyDown(window, { code: 'F6', key: 'F6' });
  expect(await screen.findByRole('alert')).toHaveProperty(
    'textContent',
    'Shift is reserved for capitals.',
  );
});

it('waits for the whole chord instead of binding the modifier that starts it', async () => {
  render(Hotkey);
  await fireEvent.click(screen.getByRole('button', { name: 'Change key' }));

  // Ctrl+Alt+M arrives as three presses, and the first two are modifiers.
  await fireEvent.keyDown(window, { code: 'ControlLeft', key: 'Control', ctrlKey: true });
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
  await fireEvent.keyDown(window, { code: 'AltLeft', key: 'Alt', ctrlKey: true, altKey: true });
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();

  await fireEvent.keyDown(window, { code: 'KeyM', key: 'm', ctrlKey: true, altKey: true });
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('audio.hotkey', 'Ctrl+Alt+M');
});

it('binds a modifier on its own once it is released with nothing against it', async () => {
  render(Hotkey);
  await fireEvent.click(screen.getByRole('button', { name: 'Change key' }));
  await fireEvent.keyDown(window, { code: 'AltRight', key: 'Alt', altKey: true });
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
  await fireEvent.keyUp(window, { code: 'AltRight', key: 'Alt' });
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('audio.hotkey', 'RightOption');
});

it('binds the modifier that was released, not one still held', async () => {
  render(Hotkey);
  await fireEvent.click(screen.getByRole('button', { name: 'Change key' }));
  await fireEvent.keyDown(window, { code: 'ControlLeft', key: 'Control', ctrlKey: true });
  await fireEvent.keyDown(window, { code: 'AltLeft', key: 'Alt', ctrlKey: true, altKey: true });
  // Control goes up while Option is still down: neither is the binding yet.
  await fireEvent.keyUp(window, { code: 'ControlLeft', key: 'Control', altKey: true });
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
});
