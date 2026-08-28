import { fireEvent, render, screen } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('../lib/tauri', () => ({
  setSetting: vi.fn(),
  status: vi.fn(),
  previewVoice: vi.fn(),
  copyText: vi.fn(),
}));
import { previewVoice, setSetting, status } from '../lib/tauri';
import ready from '../fixtures/ready.json';
import { daemon, empty, reduceStatus } from '../lib/daemon';
import { announcement, forgetCopy } from '../lib/copy';
import Voice from './Voice.svelte';

const VOICES = {
  voices: [
    { id: 'af_sky', name: 'Sky', description: 'American, clear' },
    { id: 'af_bella', name: 'Bella', description: 'American, warm' },
  ],
  current: 'af_sky',
};

beforeEach(() => {
  vi.clearAllMocks();
  forgetCopy();
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue(ready);
  vi.mocked(previewVoice).mockResolvedValue(undefined);
  daemon.set(reduceStatus(empty(), ready));
});

it('says the new voice waits on a restart, since no voice applies live', async () => {
  render(Voice, { voices: VOICES });
  await screen.findByText('Bella');
  daemon.set(reduceStatus(empty(), { ...ready, pending: ['tts.voice'] }));
  expect(await screen.findByText(/once Banshee restarts/)).toBeTruthy();
});

it('marks the voice the config names, not the one loaded at open', async () => {
  render(Voice, { voices: VOICES });
  await screen.findByText('Bella');
  daemon.set(
    reduceStatus(empty(), { ...ready, config: { ...ready.config, tts: { ...ready.config.tts, voice: 'af_bella' } } }),
  );
  const bella = await screen.findByText('Bella');
  expect(bella.getAttribute('style')).toContain('font-weight: 600');
});

it('says so when a voice will not play', async () => {
  vi.mocked(previewVoice).mockRejectedValue(new Error('no tts'));
  render(Voice, { voices: VOICES });
  await fireEvent.click(await screen.findByRole('button', { name: 'Preview Sky' }));
  await vi.waitFor(() => expect(get(announcement)).toBe('That voice will not play.'));
});

it('renders without a voice list, since the parent may not have one', async () => {
  render(Voice, { voices: { voices: [], current: null } });
  expect(await screen.findByLabelText('Speed')).toBeTruthy();
});
