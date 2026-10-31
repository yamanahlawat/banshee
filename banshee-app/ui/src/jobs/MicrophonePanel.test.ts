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
  type ListeningModel,
  type Status,
} from '../lib/daemon';
import {
  copyText,
  downloadModels,
  listDevices,
  openLicence,
  setSetting,
  status,
} from '../lib/tauri';
import {
  announcement,
  forgetCopy,
  listeningNote,
  problem,
  report,
  PENDING_SAYS,
  TAKES_EFFECT,
} from '../lib/copy';
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

const PARAKEET: ListeningModel = {
  id: 'parakeet-unified-en-0.6b',
  label: 'Parakeet Unified',
  megabytes: 731,
  downloaded: true,
  suggested: true,
  languages: ['en'],
  streams: true,
  credit:
    'Parakeet Unified by NVIDIA. Licensed by NVIDIA Corporation under the NVIDIA Open Model License.',
  licence: {
    name: 'NVIDIA Open Model License (2025-10-24)',
    summary: 'Commercial use is allowed.',
  },
  accept_command: null,
};
const TURBO: ListeningModel = {
  id: 'whisper-large-v3-turbo',
  label: 'Whisper turbo',
  megabytes: 619,
  downloaded: false,
  suggested: true,
  languages: ['af', 'am', 'ar', 'as', 'az'],
  streams: false,
  credit: null,
  licence: { name: 'MIT', summary: 'Use it.' },
  accept_command: null,
};
const GIGAAM: ListeningModel = {
  ...TURBO,
  id: 'gigaam-v3-ctc',
  label: 'gigaam-v3-ctc',
  megabytes: 225,
  downloaded: true,
  suggested: false,
  languages: ['ru'],
};
const CANARY: ListeningModel = {
  ...TURBO,
  id: 'canary-1b',
  label: 'canary-1b',
  suggested: false,
  languages: ['en', 'de', 'es', 'fr'],
  licence: {
    name: 'CC-BY-NC-4.0',
    summary: 'Use and share it, crediting its authors, for non-commercial purposes only.',
  },
  accept_command: 'banshee setup --model canary-1b --accept-licence',
};

// A local listener with a key the daemon still holds, which is the one state
// that says what stays saved.
const localStatus: Status = {
  ...withStt({ provider: 'local', model: PARAKEET.id }, LOCAL_IN_FORCE),
  stt_models: [PARAKEET, TURBO, GIGAAM],
  stt_translates: false,
};

function choosing(model: ListeningModel, models = [PARAKEET, TURBO, GIGAAM]): Status {
  return {
    ...localStatus,
    config: {
      ...localStatus.config,
      stt: { ...(localStatus.config?.stt ?? {}), model: model.id },
    },
    stt_models: models,
  };
}

function modelRow(label: string): HTMLElement {
  return screen
    .getByRole('radio', { name: new RegExp(`^${label}`) })
    .closest('.model') as HTMLElement;
}

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

// The panel reads the platform from the user agent, which jsdom names as neither.
const agent = vi.spyOn(navigator, 'userAgent', 'get');

// The panel reads the hardware as it mounts.
beforeEach(() => {
  agent.mockReset();
  forgetTheAsk();
  daemon.set(empty());
  forgetCopy();
  vi.clearAllMocks();
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue(remoteStatus);
  vi.mocked(listDevices).mockResolvedValue({ devices: [], current: null });
});

it('offers the listening models under a local listener and hides them under a remote one', () => {
  daemon.set(reduceStatus(empty(), localStatus));
  const local = render(MicrophonePanel);
  expect(local.queryByRole('radiogroup', { name: 'Listening model' })).not.toBeNull();
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
  expect(screen.queryByRole('radiogroup', { name: 'Listening model' })).toBeNull();
  expect((screen.getByRole('textbox', { name: 'Server' }) as HTMLInputElement).value).toBe(
    'https://api.groq.com/openai/v1',
  );
  expect((screen.getByRole('textbox', { name: 'Model' }) as HTMLInputElement).value).toBe(
    'whisper-large-v3-turbo',
  );
});

it('lists Parakeet Unified first, then Whisper turbo, then the downloaded models', () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  const group = screen.getByRole('radiogroup', { name: 'Listening model' });
  const labels = [...group.querySelectorAll('label')].map((one) => one.textContent?.trim());
  expect(labels).toEqual([
    'Parakeet Unified',
    'Whisper turbo, not on this machine',
    'gigaam-v3-ctc',
  ]);
  expect(
    (screen.getByRole('radio', { name: /^Parakeet Unified/ }) as HTMLInputElement).checked,
  ).toBe(true);
});

it('gives each model its size, its languages and, on macOS, whether words show as you speak', () => {
  agent.mockReturnValue('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15');
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(modelRow('Parakeet Unified').textContent).toContain(
    '731 MB. English. Shows words as you speak.',
  );
  expect(modelRow('Whisper turbo').textContent).toContain(
    '619 MB. 5 languages. Shows words once you stop.',
  );
  expect(modelRow('gigaam-v3-ctc').textContent).toContain('225 MB. Russian.');
  expect(screen.queryByText(/Many languages/i)).toBeNull();
});

it('leaves live words unsaid off macOS', () => {
  agent.mockReturnValue('Mozilla/5.0 (X11; Linux x86_64)');
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(modelRow('Parakeet Unified').textContent).toContain('731 MB. English.');
  expect(screen.queryByText(/Shows words/)).toBeNull();
});

it('credits Parakeet, and opens its licence', async () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  const parakeet = modelRow('Parakeet Unified');
  expect(parakeet.textContent).toContain(
    'Parakeet Unified by NVIDIA. Licensed by NVIDIA Corporation under the NVIDIA Open Model License.',
  );
  expect(modelRow('Whisper turbo').textContent).not.toContain('Licensed by');

  await fireEvent.click(
    screen.getByRole('button', { name: 'Read the licence for Parakeet Unified' }),
  );
  expect(vi.mocked(openLicence)).toHaveBeenCalledWith(PARAKEET.id);
});

it('shows no Language row', () => {
  daemon.set(reduceStatus(empty(), { ...localStatus, stt_translates: true }));
  render(MicrophonePanel);
  expect(screen.queryByRole('combobox', { name: 'Language' })).toBeNull();
  expect(screen.queryByText('Language')).toBeNull();
});

it('asks what to write it in only for a listener that translates', async () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(screen.queryByRole('radiogroup', { name: 'Write it in' })).toBeNull();

  daemon.set(reduceStatus(empty(), { ...localStatus, stt_translates: true }));
  await waitFor(() => expect(screen.getByRole('radiogroup', { name: 'Write it in' })).toBeTruthy());
  await fireEvent.click(screen.getByRole('radio', { name: 'English' }));
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.translate', true);
});

it('writes the model chosen, and offers its download with its size', async () => {
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();

  await fireEvent.click(screen.getByRole('radio', { name: /^Whisper turbo/ }));
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.model', TURBO.id);

  daemon.set(reduceStatus(empty(), { ...choosing(TURBO), pending: ['stt.model'] }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
  expect(
    screen.getByText('Whisper turbo is not on this machine. About 619 MB to fetch.'),
  ).toBeTruthy();
});

it('offers nothing while the chosen model is here', () => {
  daemon.set(reduceStatus(empty(), { ...localStatus, missing_downloads: [] }));
  const { container } = render(MicrophonePanel);
  expect(container.querySelector('.note.pending')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
});

// The window offers no acceptance, so a model set through the CLI names its
// licence and the command that accepts it.
it('names the licence a chosen model waits on, and the command that accepts it', async () => {
  daemon.set(reduceStatus(empty(), choosing(CANARY, [PARAKEET, TURBO, CANARY])));
  render(MicrophonePanel);

  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
  expect(
    screen.getByText(
      'Banshee will not download canary-1b until you accept its licence, CC-BY-NC-4.0.',
    ),
  ).toBeTruthy();
  expect(
    screen.getByText('Use and share it, crediting its authors, for non-commercial purposes only.'),
  ).toBeTruthy();
  expect(screen.getByText(CANARY.accept_command as string)).toBeTruthy();

  await fireEvent.click(screen.getByRole('button', { name: 'Read the licence for canary-1b' }));
  expect(vi.mocked(openLicence)).toHaveBeenCalledWith(CANARY.id);
  await fireEvent.click(screen.getByRole('button', { name: 'Copy the command' }));
  expect(vi.mocked(copyText)).toHaveBeenCalledWith(CANARY.accept_command);
});

// A Moonshine build carries a credit and waits on its licence, so the row and
// the block below would each offer the same button.
it('offers one licence button for a model whose licence waits', () => {
  const moonshine: ListeningModel = {
    ...CANARY,
    id: 'moonshine-tiny-ja',
    label: 'moonshine-tiny-ja',
    languages: ['ja'],
    credit: 'Powered by Moonshine AI',
    accept_command: 'banshee setup --model moonshine-tiny-ja --accept-licence',
  };
  daemon.set(reduceStatus(empty(), choosing(moonshine, [PARAKEET, TURBO, moonshine])));
  render(MicrophonePanel);
  expect(screen.getByText(/Powered by Moonshine AI/)).toBeTruthy();
  expect(
    screen.getAllByRole('button', { name: 'Read the licence for moonshine-tiny-ja' }),
  ).toHaveLength(1);
});

it('says where to read a licence the window could not open', async () => {
  vi.mocked(openLicence).mockRejectedValue({
    message: 'The licence did not open. Read it at https://example.com/licence.',
  });
  report('');
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  await fireEvent.click(
    screen.getByRole('button', { name: 'Read the licence for Parakeet Unified' }),
  );
  await waitFor(() =>
    expect(get(problem)).toBe('The licence did not open. Read it at https://example.com/licence.'),
  );
});

it('draws no model list while the daemon has none to offer', () => {
  daemon.set(reduceStatus(empty(), { ...localStatus, running: false, stt_models: undefined }));
  render(MicrophonePanel);
  expect(screen.queryByRole('radiogroup', { name: 'Listening model' })).toBeNull();
});

it('puts the mark back on the model in force when the daemon refuses a choice', async () => {
  vi.mocked(setSetting).mockRejectedValue({ message: 'no' });
  daemon.set(reduceStatus(empty(), localStatus));
  render(MicrophonePanel);
  const turbo = screen.getByRole('radio', { name: /^Whisper turbo/ }) as HTMLInputElement;
  await fireEvent.click(turbo);
  await waitFor(() =>
    expect(
      (screen.getByRole('radio', { name: /^Parakeet Unified/ }) as HTMLInputElement).checked,
    ).toBe(true),
  );
  expect(turbo.checked).toBe(false);
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

// Dictation carries on with the loaded model, so a chosen model that is not
// on disk raises no blocker at all.
it('offers the fetch while the daemon is working and nothing is blocked', () => {
  daemon.set(reduceStatus(empty(), { ...choosing(TURBO), ready: true, blockers: [] }));
  render(MicrophonePanel);
  expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy();
});

it('sends one download however fast the button is pressed', async () => {
  vi.mocked(downloadModels).mockResolvedValue(undefined);
  daemon.set(reduceStatus(empty(), choosing(TURBO)));
  render(MicrophonePanel);

  const fetch = screen.getByRole('button', { name: 'Download' });
  await fireEvent.click(fetch);
  await fireEvent.click(fetch);

  expect(vi.mocked(downloadModels)).toHaveBeenCalledTimes(1);
  expect((fetch as HTMLButtonElement).disabled).toBe(true);
});
