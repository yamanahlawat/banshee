import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { downloadModels, setSetting, status, type Voices } from '../lib/tauri';
import { daemon, empty } from '../lib/daemon';
import VoicePanel from './VoicePanel.svelte';

const VOICES: Voices = {
  voices: [
    { id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true },
    { id: 'am_adam', name: 'Adam', description: 'American, low', downloaded: false },
  ],
  current: 'af_sky',
};

beforeEach(() => {
  vi.mocked(setSetting).mockReset().mockResolvedValue([]);
  vi.mocked(downloadModels).mockReset().mockResolvedValue(undefined);
  vi.mocked(status).mockReset().mockResolvedValue({ running: true, config: {} });
  daemon.set(empty());
});

it('fetches nothing when the daemon refuses the voice', async () => {
  vi.mocked(setSetting).mockRejectedValue(new Error('no such voice'));
  const { getByRole } = render(VoicePanel, { voices: VOICES });

  await fireEvent.change(getByRole('radio', { name: /Adam/ }));

  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalled());
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
});

it('fetches the file when the daemon takes a voice it does not have', async () => {
  const { getByRole } = render(VoicePanel, { voices: VOICES });

  await fireEvent.change(getByRole('radio', { name: /Adam/ }));

  await waitFor(() => expect(vi.mocked(downloadModels)).toHaveBeenCalled());
});

it('fetches nothing for a voice already on the machine', async () => {
  const { getByRole } = render(VoicePanel, { voices: VOICES });

  await fireEvent.change(getByRole('radio', { name: /Sky/ }));

  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalled());
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
});
