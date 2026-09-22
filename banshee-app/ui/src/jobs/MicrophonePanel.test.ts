import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import remote from '../mocks/remote.json';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import {
  daemon,
  empty,
  listeningFacts,
  reduceStatus,
  waitsOnARestart,
  type Status,
} from '../lib/daemon';
import { downloadModels, listDevices, listLanguages, setSetting, status } from '../lib/tauri';
import { announcement, forgetCopy, listeningNote, PENDING_SAYS, TAKES_EFFECT } from '../lib/copy';
import { forgetTheAsk } from '../lib/downloads';
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
  tts: { remote: false, host: null, speaker_started: true, key_present: false },
};

// A local listener with a key the daemon still holds, which is the one state
// that says what stays saved.
const localStatus = withStt({ provider: 'local' }, LOCAL_IN_FORCE);

// The two directions the choice can wait in: the config holds one listener and
// the daemon runs the other.
const toRemote: Status = { ...withStt({}, LOCAL_IN_FORCE), pending: ['stt.provider'] };
const toLocal: Status = { ...withStt({ provider: 'local' }), pending: ['stt.provider'] };

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
  forgetTheAsk();
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
        {
          stt: { remote: true, host: 'api.groq.com', key_present: false },
          tts: { remote: false, host: null, speaker_started: true, key_present: false },
        },
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

const absentSpeech = { name: 'ggml-large-v3-q5_0.bin', role: 'speech' as const, megabytes: 1031 };

it('names what a chosen model still costs, and offers the fetch here', () => {
  daemon.set(reduceStatus(empty(), { ...localStatus, missing_downloads: [absentSpeech] }));
  const { container } = render(MicrophonePanel);
  const said = container.querySelector('.note.pending')?.textContent ?? '';
  expect(said).toContain('1.0 GB');
  expect(said).not.toBe(PENDING_SAYS);
  expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy();
});

it('offers nothing while the speech model this preset names is here', () => {
  daemon.set(reduceStatus(empty(), { ...localStatus, missing_downloads: [] }));
  const { container } = render(MicrophonePanel);
  expect(container.querySelector('.note.pending')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
});

// The run's total covers the detector, the engine and a voice, so a voice the
// reader has not fetched would otherwise price a speech model that is present.
it('says nothing about the model when only a voice is missing', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...localStatus,
      download_megabytes: 1,
      missing_downloads: [{ name: 'bf_emma.bin', role: 'voice' as const, megabytes: 1 }],
    }),
  );
  const { container } = render(MicrophonePanel);
  expect(container.querySelector('.note.pending')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
});

it('says the remote server waits on a restart', () => {
  daemon.set(reduceStatus(empty(), { ...remoteStatus, pending: ['stt.remote.base_url'] }));
  const { container } = render(MicrophonePanel);
  const pending = container.querySelectorAll('.note.pending');
  expect(pending).toHaveLength(1);
  expect(pending[0].textContent).toBe(PENDING_SAYS);
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
          tts: { remote: false, host: null, speaker_started: true, key_present: false },
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

// `copy.test.ts` pins each sentence over the pure function. This asks the one
// thing only a render answers: that the group is read with the note the facts
// the panel holds produce.
it('describes the choice by the sentence that says where audio goes', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  const described = screen
    .getByRole('radiogroup', { name: 'Listening' })
    .getAttribute('aria-describedby');
  expect(document.getElementById(described ?? '')?.textContent).toBe(
    listeningNote(listeningFacts(get(daemon), get(waitsOnARestart))),
  );
});

// The group says the restart in its own sentence, so the pending line every
// other row draws would state it a second time.
it('states the restart in the sentence, and draws no pending line for it', () => {
  daemon.set(reduceStatus(empty(), toRemote));
  render(MicrophonePanel);
  expect(
    screen.getByText(listeningNote(listeningFacts(get(daemon), get(waitsOnARestart)))),
  ).toBeTruthy();
  expect(listenerGroup().querySelectorAll('.note.pending')).toHaveLength(0);
});

// The choice is real but idle while the daemon is down, so the note may not
// claim audio goes anywhere.
it('says nothing is heard while the daemon is down', () => {
  daemon.set(reduceStatus(empty(), { ...remoteStatus, running: false }));
  render(MicrophonePanel);
  expect(screen.getByText('Nothing is heard until Banshee starts.')).toBeTruthy();
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
      `Audio still stays on this machine. ${TAKES_EFFECT} Server, key and model are below.`,
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

it('reads the group with the failure standing under it', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_error: 'the remote listener refused the key',
    }),
  );
  render(MicrophonePanel);
  const described = screen
    .getByRole('radiogroup', { name: 'Listening' })
    .getAttribute('aria-describedby');
  expect(described).toBe('listener-note dictation-failure');
  expect(document.getElementById('dictation-failure')?.textContent).toBe(
    'The last dictation failed. the remote listener refused the key',
  );
});

it('puts the key under the server it opens', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(MicrophonePanel);
  const names = [...listenerGroup().querySelectorAll('.sub')].map((name) => name.textContent);
  expect(names).toEqual(['server', 'key', 'model']);
});

// Dictation carries on with the loaded model, so a heavier preset that is not
// on disk raises no blocker at all.
it('offers the fetch while the daemon is working and nothing is blocked', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...localStatus,
      ready: true,
      blockers: [],
      missing_downloads: [absentSpeech],
    }),
  );
  render(MicrophonePanel);
  expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy();
});

// `english_only` answers for the model the listener loaded, and a heavier one
// takes seconds to read off disk.
it('says the listener has not picked the chosen model up yet, and leaves the language reachable', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...withStt({ provider: 'local', preset: 'balanced' }, LOCAL_IN_FORCE),
      english_only: true,
    }),
  );
  render(MicrophonePanel);

  expect(screen.getByText(/has not picked up Balanced yet/)).toBeTruthy();
  expect(screen.queryByText(/Fast hears English only/)).toBeNull();
  expect((screen.getByRole('combobox', { name: 'Language' }) as HTMLSelectElement).disabled).toBe(
    false,
  );
});

// Fast loaded and Fast chosen is not a gap: the model really does hear English
// alone, so the picker stays shut.
it('keeps the language shut while Fast is both chosen and loaded', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...withStt({ provider: 'local', preset: 'fast' }, LOCAL_IN_FORCE),
      english_only: true,
    }),
  );
  render(MicrophonePanel);

  expect(screen.getByText(/Fast hears English only/)).toBeTruthy();
  expect((screen.getByRole('combobox', { name: 'Language' }) as HTMLSelectElement).disabled).toBe(
    true,
  );
});

it('sends one download however fast the button is pressed', async () => {
  vi.mocked(downloadModels).mockResolvedValue(undefined);
  daemon.set(reduceStatus(empty(), { ...localStatus, missing_downloads: [absentSpeech] }));
  render(MicrophonePanel);

  const fetch = screen.getByRole('button', { name: 'Download' });
  await fireEvent.click(fetch);
  await fireEvent.click(fetch);

  expect(vi.mocked(downloadModels)).toHaveBeenCalledTimes(1);
  expect((fetch as HTMLButtonElement).disabled).toBe(true);
});
