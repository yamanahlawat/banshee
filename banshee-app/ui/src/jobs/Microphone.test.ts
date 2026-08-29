import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('../lib/tauri', () => ({
  setSetting: vi.fn(),
  status: vi.fn(),
  listDevices: vi.fn(),
  copyText: vi.fn(),
}));
import { listDevices, setSetting, status } from '../lib/tauri';
import ready from '../fixtures/ready.json';
import { daemon, empty, reduceStatus } from '../lib/daemon';
import { announcement, forgetCopy } from '../lib/copy';
import { get } from 'svelte/store';
import Microphone from './Microphone.svelte';

const withVocabulary = (words: string[]) => ({
  ...ready,
  config: { ...ready.config, stt: { ...ready.config.stt, vocabulary: words } },
});

beforeEach(() => {
  vi.clearAllMocks();
  forgetCopy();
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue(ready);
  vi.mocked(listDevices).mockResolvedValue({ devices: [{ name: 'MacBook Pro Microphone', default: true }], current: 'MacBook Pro Microphone' });
  daemon.set(reduceStatus(empty(), ready));
});

it('renders a vocabulary that repeats a word, since the file may hold one', () => {
  daemon.set(reduceStatus(empty(), withVocabulary(['Tauri', 'Tauri', 'Svelte'])));
  render(Microphone);
  expect(screen.getAllByText('Tauri').length).toBe(2);
  expect(screen.getByText('3 words.')).toBeTruthy();
});

it('removes the entry pressed, not every copy of that word', async () => {
  daemon.set(reduceStatus(empty(), withVocabulary(['Tauri', 'Tauri', 'Svelte'])));
  render(Microphone);
  await fireEvent.click(screen.getAllByRole('button', { name: 'Remove Tauri' })[0]);
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.vocabulary', ['Tauri', 'Svelte']);
});

it('adds a word the list does not already hold', async () => {
  daemon.set(reduceStatus(empty(), withVocabulary(['Tauri'])));
  render(Microphone);
  await fireEvent.click(screen.getByRole('button', { name: 'Add a word' }));
  const field = screen.getByLabelText('Add a word');
  await fireEvent.keyDown(field, { key: 'Enter', target: { value: 'Svelte' } });
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.vocabulary', ['Tauri', 'Svelte']);
});

it('says so when the daemon refuses a write', async () => {
  vi.mocked(setSetting).mockRejectedValue({ message: 'That device is gone.' });
  render(Microphone);
  await fireEvent.change(screen.getByLabelText('Sensitivity'), { target: { value: '0.8' } });
  await vi.waitFor(() => expect(get(announcement)).toBe('That device is gone.'));
});

it('opens with an empty picker rather than throwing when the daemon is gone', async () => {
  vi.mocked(listDevices).mockRejectedValue(new Error('no daemon'));
  render(Microphone);
  expect(await screen.findByLabelText('Input')).toBeTruthy();
});

// A microphone that comes or goes must move the list without the panel being
// reopened.
it('reads the devices again when the daemon reports a different one', async () => {
  render(Microphone);
  await vi.waitFor(() => expect(vi.mocked(listDevices)).toHaveBeenCalled());
  const reads = vi.mocked(listDevices).mock.calls.length;

  daemon.update((held) => ({ ...held, live: { ...held.live, audio_device: 'Studio Mic' } }));

  await vi.waitFor(() =>
    expect(vi.mocked(listDevices).mock.calls.length).toBe(reads + 1),
  );
});
