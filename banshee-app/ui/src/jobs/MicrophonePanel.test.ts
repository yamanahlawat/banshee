import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import remote from '../fixtures/remote.json';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { daemon, empty, reduceStatus, type Status } from '../lib/daemon';
import { listDevices, listLanguages, setSetting, status } from '../lib/tauri';
import { announcement, forgetCopy, PENDING_SAYS } from '../lib/copy';
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

// A local listener with a key the daemon still holds, which is the one state
// that says what stays saved.
const localStatus = withStt(
  { provider: 'local' },
  {
    stt: { remote: false, host: null, key_present: true },
    tts: { remote: false },
  },
);

// The row that holds the listener choice. The group is the design here, so a
// test asks whether the parts are inside it rather than only that they exist.
function listenerGroup(): HTMLElement {
  return screen.getByRole('radiogroup', { name: 'Listening' }).closest('.row') as HTMLElement;
}

// The panel reads the hardware and the language table as it mounts.
beforeEach(() => {
  daemon.set(empty());
  forgetCopy();
  vi.clearAllMocks();
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue(remoteStatus);
  vi.mocked(listDevices).mockResolvedValue({ devices: [], current: null });
  vi.mocked(listLanguages).mockResolvedValue({ languages: [] });
});

it('offers the preset under a local listener and hides it under a remote one', () => {
  daemon.set(reduceStatus(empty(), withStt({ provider: 'local' }, undefined)));
  const local = render(MicrophonePanel);
  expect(local.queryByRole('radiogroup', { name: 'Model' })).not.toBeNull();
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
  expect(screen.queryByRole('radiogroup', { name: 'Model' })).toBeNull();
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

it('holds the server, the model and the key inside the listener group', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  const group = listenerGroup();
  for (const name of ['Server', 'Model']) {
    expect(group.contains(screen.getByRole('textbox', { name }))).toBe(true);
  }
  expect(group.contains(document.querySelector('input[type="password"]'))).toBe(true);
});

it('describes the choice by the sentence that says where audio goes', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  const described = screen
    .getByRole('radiogroup', { name: 'Listening' })
    .getAttribute('aria-describedby');
  expect(document.getElementById(described ?? '')?.textContent).toBe(
    'Audio goes to the server below.',
  );
});

it('says what stays saved when the listener goes back to this machine', () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(
    screen.getByText('Audio stays on this machine. The server and key you set are still saved.'),
  ).toBeTruthy();
});

// The note is what the control means, and a restart notice is a second fact
// about it. Replacing the one with the other drops the first.
it('keeps the note when the provider waits on a restart', () => {
  daemon.set(reduceStatus(empty(), { ...remoteStatus, pending: ['stt.provider'] }));
  render(MicrophonePanel);
  expect(screen.getByText('Audio goes to the server below.')).toBeTruthy();
  expect(screen.getByText(PENDING_SAYS)).toBeTruthy();
});

it('removes a stored key from the window and says the restart it needs', async () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  await fireEvent.click(screen.getByRole('button', { name: 'Remove the key' }));
  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.remote.api_key', ''));
  await waitFor(() =>
    expect(get(announcement)).toBe('The key is removed. It takes effect when Banshee restarts.'),
  );
});

it('offers no removal when the daemon holds no key', () => {
  daemon.set(
    reduceStatus(
      empty(),
      withStt(
        {},
        {
          stt: { remote: true, host: 'api.openai.com', key_present: false },
          tts: { remote: false },
        },
      ),
    ),
  );
  render(MicrophonePanel);
  expect(screen.queryByRole('button', { name: 'Remove the key' })).toBeNull();
});

// A screen reader hears the fields appear from nothing otherwise.
it('announces the rows a change of provider discloses, and nothing on arrival', async () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(get(announcement)).toBe('');

  daemon.set(reduceStatus(empty(), remoteStatus));
  await waitFor(() => expect(get(announcement)).toBe('Server, model and key are below.'));
});

it('agrees with the count when the quiet is one second', () => {
  daemon.set(reduceStatus(empty(), withStt({ endpoint_silence_ms: 1000 }, undefined)));
  render(MicrophonePanel);
  expect(screen.getByRole('option', { name: 'After 1 second of quiet' })).toBeTruthy();
  expect(screen.getByRole('option', { name: 'After 2.5 seconds of quiet' })).toBeTruthy();
});

it('names the last failure inside the group that caused it', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_error: 'the remote listener refused the key',
    }),
  );
  render(MicrophonePanel);
  const failure = screen.getByText(/refused the key/);
  expect(listenerGroup().contains(failure)).toBe(true);
});
