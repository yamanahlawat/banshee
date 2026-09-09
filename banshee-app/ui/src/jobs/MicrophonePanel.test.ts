import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import remote from '../fixtures/remote.json';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { daemon, empty, reduceStatus, type Status } from '../lib/daemon';
import { listDevices, listLanguages, setSetting, status } from '../lib/tauri';
import { announcement, forgetCopy } from '../lib/copy';
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

const LOCAL_IN_FORCE: Status['remote'] = {
  stt: { remote: false, host: null, key_present: true },
  tts: { remote: false },
};

// A local listener with a key the daemon still holds, which is the one state
// that says what stays saved.
const localStatus = withStt({ provider: 'local' }, LOCAL_IN_FORCE);

// The two directions the choice can wait in: the config holds one listener and
// the daemon runs the other.
const toRemote: Status = { ...withStt({}, LOCAL_IN_FORCE), pending: ['stt.provider'] };
const toLocal: Status = { ...withStt({ provider: 'local' }), pending: ['stt.provider'] };

// The one sentence the group says about the restart it waits on.
const TAKES_EFFECT = 'Your choice takes effect when Banshee restarts.';

function keyField(): HTMLInputElement | null {
  return document.querySelector('input[type="password"]');
}

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

it('holds the server, the model and the key inside the listener group', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  const group = listenerGroup();
  for (const name of ['Server', 'Model']) {
    expect(group.contains(screen.getByRole('textbox', { name }))).toBe(true);
  }
  expect(group.contains(screen.getByText('A key is set'))).toBe(true);
});

// A masked field whose placeholder carried the state could not be told from a
// field holding a value, so the state is a line of text and the key row is two
// forms rather than one.
it('says a key is set, and never the key', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  expect(screen.getByText('A key is set')).toBeTruthy();
  expect(keyField()).toBeNull();
  expect(screen.getByRole('button', { name: 'Remove the key' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Replace the key' })).toBeTruthy();
});

it('draws the key as missing when the daemon holds none', () => {
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
  const key = keyField() as HTMLInputElement;
  expect(key.placeholder).toBe('Paste a key');
  expect(key.value).toBe('');
  // jsdom resolves no scoped stylesheet, so the class is the only part of the
  // dashed mark a test here can read.
  expect(key.classList.contains('dashed')).toBe(true);
  expect(screen.queryByRole('button', { name: 'Remove the key' })).toBeNull();
});

it('swaps a field in to replace the key, and takes the line back when it commits', async () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  await fireEvent.click(screen.getByRole('button', { name: 'Replace the key' }));

  const key = (await waitFor(() => keyField())) as HTMLInputElement;
  expect(key.classList.contains('dashed')).toBe(false);
  expect(document.activeElement).toBe(key);

  await fireEvent.input(key, { target: { value: 'sk-second' } });
  await fireEvent.blur(key);
  await waitFor(() =>
    expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.remote.api_key', 'sk-second'),
  );
  await waitFor(() => expect(screen.getByText('A key is set')).toBeTruthy());
});

it('describes the choice by the sentence that says where audio goes', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  const described = screen
    .getByRole('radiogroup', { name: 'Listening' })
    .getAttribute('aria-describedby');
  expect(document.getElementById(described ?? '')?.textContent).toBe(
    'Audio goes to api.openai.com.',
  );
});

it('says what stays saved when the listener goes back to this machine', () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(
    screen.getByText('Audio stays on this machine. The server and key you set are still saved.'),
  ).toBeTruthy();
});

// The group held two truths from two sources and stated the restart four times.
// One sentence now reads the daemon for what is in force and the config for
// what was asked, so the group cannot contradict itself.
it('states where audio goes and the restart it waits on in one sentence', () => {
  daemon.set(reduceStatus(empty(), toRemote));
  render(MicrophonePanel);
  expect(screen.getByText(`Audio still stays on this machine. ${TAKES_EFFECT}`)).toBeTruthy();
  expect(listenerGroup().querySelectorAll('.note.pending')).toHaveLength(0);
});

it('says the audio still goes to the server while the way back waits', () => {
  daemon.set(reduceStatus(empty(), toLocal));
  render(MicrophonePanel);
  expect(screen.getByText(`Audio still goes to api.openai.com. ${TAKES_EFFECT}`)).toBeTruthy();
});

it('names a remote listener the daemon cannot name', () => {
  daemon.set(
    reduceStatus(
      empty(),
      // An empty host is what the daemon answers when the address is no URL.
      withStt({}, { stt: { remote: true, host: '', key_present: true }, tts: { remote: false } }),
    ),
  );
  render(MicrophonePanel);
  expect(screen.getByText('Audio goes to a remote server.')).toBeTruthy();
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

// A screen reader hears the fields appear from nothing otherwise, and the
// layout is the smaller half of what changed: the audio now leaves the machine.
it('announces where audio goes at the flip, and nothing on arrival', async () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(get(announcement)).toBe('');

  daemon.set(reduceStatus(empty(), toRemote));
  await waitFor(() =>
    expect(get(announcement)).toBe(
      `Audio still stays on this machine. ${TAKES_EFFECT} Server, model and key are below.`,
    ),
  );
});

it('announces the way back, and what it leaves below', async () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);

  daemon.set(reduceStatus(empty(), toLocal));
  await waitFor(() =>
    expect(get(announcement)).toBe(
      `Audio still goes to api.openai.com. ${TAKES_EFFECT} Model is below.`,
    ),
  );
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
  // A live region on an element that arrives with its own content is not
  // announced, so the failure is spoken instead.
  expect(failure.getAttribute('role')).toBeNull();
});

// The reader is most often not looking at the screen when one of these lands.
it('speaks a failure that arrives, and says nothing about one already there', async () => {
  daemon.set(
    reduceStatus(empty(), { ...remoteStatus, last_error: 'the remote listener refused the key' }),
  );
  render(MicrophonePanel);
  expect(get(announcement)).toBe('');

  daemon.set(reduceStatus(empty(), { ...remoteStatus, last_error: 'the server timed out' }));
  await waitFor(() =>
    expect(get(announcement)).toBe('The last dictation failed: the server timed out.'),
  );
});
