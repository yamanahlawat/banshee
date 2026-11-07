import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { forgetCopy, problem } from '../lib/copy';
import { stubMedia } from '../lib/media.mock';
import Player from './Player.svelte';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { historyAudio } from '../lib/tauri';

let media: ReturnType<typeof stubMedia>;

beforeEach(() => {
  forgetCopy();
  vi.mocked(historyAudio).mockReset();
  vi.mocked(historyAudio).mockResolvedValue(new ArrayBuffer(8));
  media = stubMedia();
});

afterEach(() => {
  media.restore();
});

const audio = (container: HTMLElement) => container.querySelector('audio')!;

it('names the length and flips to pause', async () => {
  const { container } = render(Player, { id: 4, seconds: 5.6 });
  await fireEvent.click(screen.getByRole('button', { name: 'Play audio, 6 seconds' }));
  expect(await screen.findByRole('button', { name: 'Pause audio' })).toBeTruthy();

  await fireEvent.click(screen.getByRole('button', { name: 'Pause audio' }));
  expect(screen.getByRole('button', { name: 'Play audio, 6 seconds' })).toBeTruthy();

  await fireEvent.click(screen.getByRole('button', { name: 'Play audio, 6 seconds' }));
  await screen.findByRole('button', { name: 'Pause audio' });
  await fireEvent(audio(container), new Event('ended'));
  expect(screen.getByRole('button', { name: 'Play audio, 6 seconds' })).toBeTruthy();
});

it('names a clip under a second as one second', () => {
  render(Player, { id: 4, seconds: 0.3 });
  expect(screen.getByRole('button', { name: 'Play audio, 1 second' })).toBeTruthy();
});

it('fetches once and reuses the url', async () => {
  const { container } = render(Player, { id: 4, seconds: 2 });
  const button = screen.getByRole('button');
  await fireEvent.click(button);
  await screen.findByRole('button', { name: 'Pause audio' });
  await fireEvent.click(button);
  await fireEvent.click(button);
  await screen.findByRole('button', { name: 'Pause audio' });

  expect(historyAudio).toHaveBeenCalledOnce();
  expect(historyAudio).toHaveBeenCalledWith(4);
  expect(URL.createObjectURL).toHaveBeenCalledOnce();
  const blob = vi.mocked(URL.createObjectURL).mock.calls[0][0] as Blob;
  expect(blob.type).toBe('audio/wav');
  expect(audio(container).src).toBe('blob:clip-1');
  expect(media.play).toHaveBeenCalledTimes(2);
});

it('revokes the url on destroy', async () => {
  const { unmount } = render(Player, { id: 4, seconds: 2 });
  await fireEvent.click(screen.getByRole('button'));
  await screen.findByRole('button', { name: 'Pause audio' });
  unmount();
  expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:clip-1');
  expect(media.pause).toHaveBeenCalled();
});

it('a refused fetch is reported', async () => {
  vi.mocked(historyAudio).mockRejectedValue({ message: 'No audio is kept for this row.' });
  render(Player, { id: 4, seconds: 2 });
  await fireEvent.click(screen.getByRole('button'));
  await waitFor(() => expect(get(problem)).toBe('No audio is kept for this row.'));
  expect(media.play).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: 'Play audio, 2 seconds' })).toBeTruthy();
});

it('reports no refusal after it is gone', async () => {
  let refuse: (reason: unknown) => void = () => {};
  vi.mocked(historyAudio).mockReturnValueOnce(new Promise((_, reject) => (refuse = reject)));
  const { unmount } = render(Player, { id: 4, seconds: 2 });
  await fireEvent.click(screen.getByRole('button'));
  unmount();
  refuse({ message: 'No audio is kept for this row.' });
  await new Promise((resolve) => setTimeout(resolve));
  expect(get(problem)).toBe('');
});

it.each([
  ['NotSupportedError', 'This machine cannot play the audio.'],
  ['NotAllowedError', 'The audio did not play.'],
])('a play refused with %s says so', async (name, says) => {
  media.play.mockRejectedValueOnce(new DOMException('refused', name));
  render(Player, { id: 4, seconds: 2 });
  await fireEvent.click(screen.getByRole('button'));
  await waitFor(() => expect(get(problem)).toBe(says));
});

it('a play cut short by a pause reports nothing', async () => {
  media.play.mockRejectedValueOnce(new DOMException('paused', 'AbortError'));
  render(Player, { id: 4, seconds: 2 });
  await fireEvent.click(screen.getByRole('button'));
  await new Promise((resolve) => setTimeout(resolve));
  expect(get(problem)).toBe('');
});

it('says this machine cannot play the audio on a media error', async () => {
  const { container } = render(Player, { id: 4, seconds: 2 });
  await fireEvent.click(screen.getByRole('button'));
  await screen.findByRole('button', { name: 'Pause audio' });
  await fireEvent(audio(container), new Event('error'));
  expect(get(problem)).toBe('This machine cannot play the audio.');
  expect(screen.getByRole('button', { name: 'Play audio, 2 seconds' })).toBeTruthy();
});

it('toggle plays and pauses as the button does', async () => {
  const { component } = render(Player, { id: 4, seconds: 2 });
  await component.toggle();
  expect(await screen.findByRole('button', { name: 'Pause audio' })).toBeTruthy();
  await component.toggle();
  expect(screen.getByRole('button', { name: 'Play audio, 2 seconds' })).toBeTruthy();
});
