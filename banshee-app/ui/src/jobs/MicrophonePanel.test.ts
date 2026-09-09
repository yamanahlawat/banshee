import { render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import remote from '../fixtures/remote.json';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { daemon, empty, reduceStatus, type Status } from '../lib/daemon';
import { listDevices, listLanguages } from '../lib/tauri';
import MicrophonePanel from './MicrophonePanel.svelte';

// The daemon's own reply under a remote listener, so each test states only what
// it changes.
const remoteStatus = remote as unknown as Status;

function withStt(stt: Record<string, unknown>, flags?: Status['remote']): Status {
  return {
    ...remoteStatus,
    config: {
      ...remoteStatus.config,
      stt: { ...(remoteStatus.config?.stt ?? {}), ...stt },
    },
    remote: flags ?? remoteStatus.remote,
  };
}

// The panel reads the hardware and the language table as it mounts.
beforeEach(() => {
  daemon.set(empty());
  vi.mocked(listDevices).mockResolvedValue({ devices: [], current: null });
  vi.mocked(listLanguages).mockResolvedValue({ languages: [] });
});

it('offers the preset under a local listener and hides it under a remote one', () => {
  daemon.set(reduceStatus(empty(), withStt({ provider: 'local' }, undefined)));
  const local = render(MicrophonePanel);
  expect(local.queryByRole('radiogroup', { name: 'Transcription' })).not.toBeNull();
  local.unmount();

  daemon.set(
    reduceStatus(
      empty(),
      withStt(
        {
          provider: 'remote',
          remote: { base_url: 'https://api.groq.com/openai/v1', model: 'whisper-large-v3-turbo' },
        },
        { stt: { remote: true, host: 'api.groq.com', key_present: false }, tts: { remote: false } },
      ),
    ),
  );
  render(MicrophonePanel);
  expect(screen.queryByRole('radiogroup', { name: 'Transcription' })).toBeNull();
  expect((screen.getByRole('textbox', { name: 'Server' }) as HTMLInputElement).value).toBe(
    'https://api.groq.com/openai/v1',
  );
  expect((screen.getByRole('textbox', { name: 'Model' }) as HTMLInputElement).value).toBe(
    'whisper-large-v3-turbo',
  );
});

it('shows whether a key is set and never the key', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  const { container } = render(MicrophonePanel);
  const key = container.querySelector('input[type="password"]') as HTMLInputElement;
  expect(key.placeholder).toBe('Set');
  expect(key.value).toBe('');
});

it('names the last failure when the daemon reports one', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_error: 'the remote listener refused the key',
    }),
  );
  render(MicrophonePanel);
  expect(screen.getByText(/refused the key/)).toBeTruthy();
});
