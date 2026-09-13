import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { downloadModels, setSetting, status, type Voices } from '../lib/tauri';
import {
  daemon,
  empty,
  reduceStatus,
  speechFacts,
  waitsOnARestart,
  type Status,
} from '../lib/daemon';
import { announcement, forgetCopy, speechNote, PENDING_SAYS } from '../lib/copy';
import VoicePanel from './VoicePanel.svelte';

const VOICES: Voices = {
  voices: [
    { id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true },
    { id: 'am_adam', name: 'Adam', description: 'American, low', downloaded: false },
  ],
  current: 'af_sky',
};

const LOCAL: Status = {
  running: true,
  config: { tts: { provider: 'local', voice: 'af_sky', speed: 1.2, remote: {} } },
  remote: {
    stt: { remote: false, host: null, key_present: false },
    tts: { remote: false, host: null, speaker_started: true, key_present: false },
  },
};

function speaking(tts: Record<string, unknown>, flags?: Status['remote']): Status {
  return {
    ...LOCAL,
    config: { tts: { ...(LOCAL.config?.tts ?? {}), ...tts } },
    remote: flags ?? LOCAL.remote,
  };
}

const REMOTE_IN_FORCE: Status['remote'] = {
  stt: { remote: false, host: null, key_present: false },
  tts: { remote: true, host: 'api.openai.com', speaker_started: true, key_present: true },
};

const remoteStatus = speaking(
  {
    provider: 'remote',
    remote: {
      base_url: 'https://api.openai.com/v1',
      model: 'gpt-4o-mini-tts',
      voice: 'marin',
      instructions: '',
    },
  },
  REMOTE_IN_FORCE,
);

function speakingGroup(): HTMLElement {
  return screen.getByRole('radiogroup', { name: 'Speaking' }).closest('.row') as HTMLElement;
}

function keyField(): HTMLInputElement | null {
  return document.querySelector('input[type="password"]');
}

beforeEach(() => {
  daemon.set(empty());
  forgetCopy();
  vi.clearAllMocks();
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(downloadModels).mockResolvedValue(undefined);
  vi.mocked(status).mockResolvedValue(LOCAL);
});

it('offers the voice grid under a local speaker and the server fields under a remote one', () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  const local = render(VoicePanel, { voices: VOICES });
  expect(local.getByRole('radio', { name: /Sky/ })).toBeTruthy();
  expect(local.queryByRole('textbox', { name: 'Server' })).toBeNull();
  local.unmount();

  daemon.set(reduceStatus(empty(), remoteStatus));
  render(VoicePanel, { voices: VOICES });
  expect(screen.queryByRole('radio', { name: /Sky/ })).toBeNull();
  expect((screen.getByRole('textbox', { name: 'Server' }) as HTMLInputElement).value).toBe(
    'https://api.openai.com/v1',
  );
  expect((screen.getByRole('textbox', { name: 'Model' }) as HTMLInputElement).value).toBe(
    'gpt-4o-mini-tts',
  );
  expect((screen.getByRole('textbox', { name: 'Voice' }) as HTMLInputElement).value).toBe('marin');
  expect(screen.getByRole('textbox', { name: 'Tone' })).toBeTruthy();
});

it('holds every remote field inside the speaking group', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(VoicePanel, { voices: VOICES });
  const group = speakingGroup();
  for (const name of ['Server', 'Model', 'Voice', 'Tone']) {
    expect(group.contains(screen.getByRole('textbox', { name }))).toBe(true);
  }
  expect(group.contains(screen.getByText('A key is set'))).toBe(true);
});

// The endpoint has no call that lists voices, so the field says what to type.
it('asks for a voice the server names when none is set', () => {
  daemon.set(
    reduceStatus(
      empty(),
      speaking(
        {
          provider: 'remote',
          remote: { base_url: 'https://api.openai.com/v1', model: 'tts-1', voice: '' },
        },
        {
          stt: { remote: false, host: null, key_present: false },
          tts: { remote: true, host: 'api.openai.com', speaker_started: false, key_present: false },
        },
      ),
    ),
  );
  render(VoicePanel, { voices: VOICES });
  const voice = screen.getByRole('textbox', { name: 'Voice' }) as HTMLInputElement;
  expect(voice.placeholder).toBe('A voice the server names');
  expect(voice.classList.contains('dashed')).toBe(true);
  const key = keyField() as HTMLInputElement;
  expect(key.placeholder).toBe('Paste a key');
});

// The dash says the speaker needs a value it does not have. Tone is optional,
// so it carries no dash, and its placeholder says which of the two it is.
it('marks the fields the speaker needs, and says tone is not one', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(VoicePanel, { voices: VOICES });
  const tone = screen.getByRole('textbox', { name: 'Tone' }) as HTMLInputElement;
  expect(tone.placeholder).toBe('Optional');
  expect(tone.classList.contains('dashed')).toBe(false);
});

it('says a key is set, and never the key', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(VoicePanel, { voices: VOICES });
  expect(screen.getByText('A key is set')).toBeTruthy();
  expect(keyField()).toBeNull();
  expect(screen.getByRole('button', { name: 'Remove the key' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Replace the key' })).toBeTruthy();
});

// `copy.test.ts` pins each sentence over the pure function. This asks the one
// thing only a render answers: that the group is read with the note the facts
// the panel holds produce.
it('describes the choice by the sentence that says where text goes', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(VoicePanel, { voices: VOICES });
  const described = screen
    .getByRole('radiogroup', { name: 'Speaking' })
    .getAttribute('aria-describedby');
  expect(document.getElementById(described ?? '')?.textContent).toBe(
    speechNote(speechFacts(get(daemon), get(waitsOnARestart), VOICES.voices)),
  );
});

// The choice is real but idle while the daemon is down, so the note may not
// claim text goes anywhere.
it('says nothing is spoken while the daemon is down', () => {
  daemon.set(reduceStatus(empty(), { ...remoteStatus, running: false }));
  render(VoicePanel, { voices: VOICES });
  expect(screen.getByText('Nothing is spoken until Banshee starts.')).toBeTruthy();
});

// `Row` draws the one line that says a value waits on a restart. This is the
// suite's only caller that proves it renders, rather than that it does not.
it('says the speaking rate waits on a restart', () => {
  daemon.set(reduceStatus(empty(), { ...LOCAL, pending: ['tts.speed'] }));
  const { container } = render(VoicePanel, { voices: VOICES });
  const pending = container.querySelectorAll('.note.pending');
  expect(pending).toHaveLength(1);
  expect(pending[0].textContent).toBe(PENDING_SAYS);
});

// The daemon refuses a voice whose file has not landed, and a refusal is
// exactly when the reader is left with a mark they did not move and no
// sentence. The line belongs at the voices, not at the group above them.
it('says the voice waits on a restart, beside the voices', () => {
  daemon.set(reduceStatus(empty(), { ...LOCAL, pending: ['tts.voice'] }));
  const { container } = render(VoicePanel, { voices: VOICES });
  const pending = container.querySelectorAll('.note.pending');
  expect(pending).toHaveLength(1);
  expect(pending[0].textContent).toBe(PENDING_SAYS);
  expect(screen.getByRole('radio', { name: /Sky/ }).closest('.part')?.contains(pending[0])).toBe(
    true,
  );
});

// The group says the restart in its own sentence, so the pending line every
// other row draws would state it a second time.
it('states the restart in the sentence, and draws no pending line for it', () => {
  daemon.set(reduceStatus(empty(), { ...remoteStatus, pending: ['tts.provider'] }));
  const { container } = render(VoicePanel, { voices: VOICES });
  expect(
    screen.getByText(speechNote(speechFacts(get(daemon), get(waitsOnARestart), VOICES.voices))),
  ).toBeTruthy();
  expect(container.querySelectorAll('.note.pending')).toHaveLength(0);
});

it('names the last spoken failure inside the group that caused it', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_speech_error: 'the remote speaker refused the key',
    }),
  );
  render(VoicePanel, { voices: VOICES });
  const failure = screen.getByText(
    'The last spoken reply failed: the remote speaker refused the key.',
  );
  expect(speakingGroup().contains(failure)).toBe(true);
});

// A failure already standing when the panel opens raises no announcement, so
// the group has to be read with it or a screen reader never meets it.
it('reads the group with the failure standing under it', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_speech_error: 'the remote speaker refused the key',
    }),
  );
  render(VoicePanel, { voices: VOICES });
  const described = screen
    .getByRole('radiogroup', { name: 'Speaking' })
    .getAttribute('aria-describedby');
  expect(described).toBe('speaker-note speech-failure');
  expect(document.getElementById('speech-failure')?.textContent).toBe(
    'The last spoken reply failed: the remote speaker refused the key.',
  );
});

// The reader is most often not looking at the screen when one of these lands.
it('speaks a failure that arrives, and says nothing about one already there', async () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_speech_error: 'the remote speaker refused the key',
    }),
  );
  render(VoicePanel, { voices: VOICES });
  expect(get(announcement)).toBe('');

  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_speech_error: 'api.openai.com did not answer in time',
    }),
  );
  await waitFor(() =>
    expect(get(announcement)).toBe(
      'The last spoken reply failed: api.openai.com did not answer in time.',
    ),
  );
});

it('removes a stored key and says the restart it needs', async () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(VoicePanel, { voices: VOICES });
  await fireEvent.click(screen.getByRole('button', { name: 'Remove the key' }));
  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.remote.api_key', ''));
  await waitFor(() =>
    expect(get(announcement)).toBe('The key is removed. It takes effect when Banshee restarts.'),
  );
});

it('fetches nothing when the daemon refuses the voice', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  vi.mocked(setSetting).mockRejectedValue(new Error('no such voice'));
  render(VoicePanel, { voices: VOICES });

  await fireEvent.change(screen.getByRole('radio', { name: /Adam/ }));

  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalled());
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
});

it('fetches the file when the daemon takes a voice it does not have', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  render(VoicePanel, { voices: VOICES });

  await fireEvent.change(screen.getByRole('radio', { name: /Adam/ }));

  await waitFor(() => expect(vi.mocked(downloadModels)).toHaveBeenCalled());
});

it('fetches nothing for a voice already on the machine', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  render(VoicePanel, { voices: VOICES });

  await fireEvent.change(screen.getByRole('radio', { name: /Sky/ }));

  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalled());
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
});

// The Speaking rate applies to both speakers, so it stays outside the group.
it('keeps the speaking rate outside the group', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(VoicePanel, { voices: VOICES });
  const rate = screen.getByRole('slider', { name: 'Speaking rate' });
  expect(speakingGroup().contains(rate)).toBe(false);
});
