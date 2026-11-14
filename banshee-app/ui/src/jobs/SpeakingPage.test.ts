import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { tick } from 'svelte';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());
vi.mock(import('../lib/travel'), async (actual) => ({
  ...(await actual()),
  travel: vi.fn(),
}));

import { copyText, downloadModels, setSetting, status, type Voices } from '../lib/tauri';
import {
  daemon,
  empty,
  reduceStatus,
  speechFacts,
  type Progress,
  type Status,
} from '../lib/daemon';
import {
  announcement,
  forgetCopy,
  problem,
  speechNote,
  WAITING_SAYS,
  WAITS_FOR_ITS_FILE,
} from '../lib/copy';
import { forgetTheAsk } from '../lib/downloads';
import { travel } from '../lib/travel';
import { forgetRefusals, table } from '../lib/settings';
import { commonTable } from '../lib/tauri.mock';
import { expectInOrder } from '../lib/order.mock';
import SpeakingPage from './SpeakingPage.svelte';

const VOICES: Voices = {
  voices: [
    {
      id: 'companion',
      name: 'Companion',
      description: 'Pocket TTS by Kyutai, CC-BY-4.0',
      downloaded: false,
    },
    { id: 'af_sky', name: 'Sky', description: 'American, clear', downloaded: true },
    { id: 'af_bella', name: 'Bella', description: 'American, warm', downloaded: false },
    { id: 'af_heart', name: 'Heart', description: 'American, soft', downloaded: true },
    { id: 'am_adam', name: 'Adam', description: 'American, low', downloaded: false },
  ],
  current: 'af_sky',
};

const LOCAL: Status = {
  running: true,
  config: { tts: { provider: 'local', voice: 'af_sky', remote: {} } },
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
  return screen.getByRole('radiogroup', { name: 'Speaking' }).closest('.apart') as HTMLElement;
}

function keyField(): HTMLInputElement | null {
  return document.querySelector('input[type="password"]');
}

beforeEach(() => {
  table.set(commonTable);
  daemon.set(empty());
  forgetCopy();
  forgetTheAsk();
  forgetRefusals();
  vi.clearAllMocks();
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(downloadModels).mockResolvedValue(undefined);
  vi.mocked(status).mockResolvedValue(LOCAL);
});

it('offers the voice grid under a local speaker and the server fields under a remote one', () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  const local = render(SpeakingPage, { voices: VOICES });
  expect(local.getByRole('radio', { name: /Sky/ })).toBeTruthy();
  expect(local.queryByRole('textbox', { name: 'Server' })).toBeNull();
  local.unmount();

  daemon.set(reduceStatus(empty(), remoteStatus));
  render(SpeakingPage, { voices: VOICES });
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
  render(SpeakingPage, { voices: VOICES });
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
  render(SpeakingPage, { voices: VOICES });
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
  render(SpeakingPage, { voices: VOICES });
  const tone = screen.getByRole('textbox', { name: 'Tone' }) as HTMLInputElement;
  expect(tone.placeholder).toBe('Optional');
  expect(tone.classList.contains('dashed')).toBe(false);
});

it('says a key is set, and never the key', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(SpeakingPage, { voices: VOICES });
  expect(screen.getByText('A key is set')).toBeTruthy();
  expect(keyField()).toBeNull();
  expect(screen.getByRole('button', { name: 'Remove the key' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Replace the key' })).toBeTruthy();
});

// `copy.test.ts` pins each sentence over the pure function. This asks the one
// thing only a render answers: that the group is read with the note the facts
// the page holds produce.
it('describes the choice by the sentence that says where text goes', () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(SpeakingPage, { voices: VOICES });
  const described = screen
    .getByRole('radiogroup', { name: 'Speaking' })
    .getAttribute('aria-describedby');
  expect(document.getElementById(described ?? '')?.textContent).toBe(
    speechNote(speechFacts(get(daemon), VOICES.voices)),
  );
});

// The choice is real but idle while the daemon is down, so the note may not
// claim text goes anywhere.
it('says nothing is spoken while the daemon is down', () => {
  daemon.set(reduceStatus(empty(), { ...remoteStatus, running: false }));
  render(SpeakingPage, { voices: VOICES });
  expect(screen.getByText('Nothing is spoken until Banshee starts.')).toBeTruthy();
});

// The daemon refuses a voice whose file has not landed, and a refusal is
// exactly when the reader is left with a mark they did not move and no
// sentence. The line belongs at the voices, not at the group above them.
it('says the voice waits for its file, beside the voices', () => {
  daemon.set(reduceStatus(empty(), { ...LOCAL, pending: ['tts.voice'] }));
  const { container } = render(SpeakingPage, { voices: VOICES });
  const pending = container.querySelectorAll('.note.pending');
  expect(pending).toHaveLength(1);
  expect(pending[0].textContent).toBe(WAITS_FOR_ITS_FILE);
  const group = screen.getByRole('radiogroup', { name: 'Voice' });
  expect(pending[0].closest('.row')).toBe(group.closest('.row'));
  expectInOrder(
    screen.getByText(speechNote(speechFacts(get(daemon), VOICES.voices))),
    screen.getByText('Voice'),
    group,
    pending[0],
  );
});

it('says the remote server is set but not in use yet', () => {
  daemon.set(reduceStatus(empty(), { ...remoteStatus, pending: ['tts.remote.base_url'] }));
  const { container } = render(SpeakingPage, { voices: VOICES });
  const pending = container.querySelectorAll('.note.pending');
  expect(pending).toHaveLength(1);
  expect(pending[0].textContent).toBe(WAITING_SAYS);
});

it('names the last spoken failure inside the group that caused it', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_speech_error: 'the remote speaker refused the key',
    }),
  );
  render(SpeakingPage, { voices: VOICES });
  const failure = screen.getByText('The last spoken reply failed.');
  expect(speakingGroup().contains(failure)).toBe(true);
  expect(speakingGroup().textContent).toContain('the remote speaker refused the key');
});

it('copies both halves of the failure', async () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_speech_error: 'the remote speaker refused the key',
    }),
  );
  render(SpeakingPage, { voices: VOICES });
  const copy = screen.getByRole('button', { name: 'Copy the failure' });
  await fireEvent.click(copy);
  expect(vi.mocked(copyText)).toHaveBeenCalledWith(
    'The last spoken reply failed. the remote speaker refused the key',
  );
  await waitFor(() => expect(copy.textContent).toContain('Copied'));
});

it('reads the group with the failure standing under it', () => {
  daemon.set(
    reduceStatus(empty(), {
      ...remoteStatus,
      last_speech_error: 'the remote speaker refused the key',
    }),
  );
  render(SpeakingPage, { voices: VOICES });
  const described = screen
    .getByRole('radiogroup', { name: 'Speaking' })
    .getAttribute('aria-describedby');
  expect(described).toBe('speaker-note speech-failure');
  expect(document.getElementById('speech-failure')?.textContent).toBe(
    'The last spoken reply failed. the remote speaker refused the key',
  );
});

it('removes a stored key and says so', async () => {
  daemon.set(reduceStatus(empty(), remoteStatus));
  render(SpeakingPage, { voices: VOICES });
  await fireEvent.click(screen.getByRole('button', { name: 'Remove the key' }));
  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.remote.api_key', ''));
  await waitFor(() => expect(get(announcement)).toBe('The key is removed.'));
});

it('fetches nothing when the daemon refuses the voice, and says so under its Get', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  vi.mocked(setSetting).mockRejectedValue({ code: -32000, message: "'tts.voice': no such voice" });
  render(SpeakingPage, { voices: VOICES });

  await fireEvent.click(screen.getByRole('button', { name: 'Get Adam and switch to it' }));

  const refusal = await waitFor(() => screen.getByText('Not changed. No such voice.'));
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
  const more = screen.getByRole('list', { name: 'More voices' });
  const adam = within(more).getByRole('button', { name: 'Get Adam and switch to it' });
  expect(refusal.closest('li')).toBe(adam.closest('li'));
  expect(adam.getAttribute('aria-describedby')?.split(' ')).toContain(refusal.id);
  expect(screen.queryByRole('radio', { name: /Adam/ })).toBeNull();
  expect(screen.getByRole('radiogroup', { name: 'Voice' }).hasAttribute('aria-describedby')).toBe(
    false,
  );
});

it('puts the mark back on the voice in force and says the refusal on its row', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  vi.mocked(setSetting).mockRejectedValue({ code: -32000, message: "'tts.voice': no such voice" });
  render(SpeakingPage, { voices: VOICES });
  const other = screen.getByRole('radio', { name: /Heart/ }) as HTMLInputElement;
  await fireEvent.click(other);

  const refusal = await waitFor(() => screen.getByText('Not changed. No such voice.'));
  expect((screen.getByRole('radio', { name: /Sky/ }) as HTMLInputElement).checked).toBe(true);
  expect(other.checked).toBe(false);
  const group = screen.getByRole('radiogroup', { name: 'Voice' });
  expect(group.getAttribute('aria-describedby')).toBe(refusal.id);
  expect(refusal.closest('.row')?.contains(group)).toBe(true);
  expect(
    screen.getByRole('list', { name: 'More voices' }).getAttribute('aria-describedby'),
  ).not.toContain(refusal.id);
});

it('fetches the file when the reader asks for a voice the machine lacks', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  render(SpeakingPage, { voices: VOICES });

  await fireEvent.click(screen.getByRole('button', { name: 'Get Adam and switch to it' }));

  await waitFor(() => expect(vi.mocked(downloadModels)).toHaveBeenCalled());
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.voice', 'am_adam');
});

function remoteAsking(format?: string, rate?: number | null): Status {
  const table = (remoteStatus.config?.tts as { remote: Record<string, unknown> }).remote;
  return speaking(
    { provider: 'remote', remote: { ...table, response_format: format, sample_rate: rate } },
    REMOTE_IN_FORCE,
  );
}

function rateField(): HTMLInputElement | null {
  return screen.queryByRole('textbox', { name: 'Sample rate' });
}

it('asks for WAV until the config names another format', () => {
  daemon.set(reduceStatus(empty(), remoteAsking(undefined)));
  render(SpeakingPage, { voices: VOICES });
  const format = screen.getByRole('radiogroup', { name: 'Audio format' });
  expect(speakingGroup().contains(format)).toBe(true);
  expect(screen.getByRole('radio', { name: 'WAV' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByRole('radio', { name: 'PCM' }).getAttribute('aria-checked')).toBe('false');
});

it('writes the format the reader picks', async () => {
  daemon.set(reduceStatus(empty(), remoteAsking('wav')));
  render(SpeakingPage, { voices: VOICES });
  await fireEvent.click(screen.getByRole('radio', { name: 'PCM' }));
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.remote.response_format', 'pcm');
});

// A WAV header states its own rate, so a rate beside it would say nothing.
it('offers a rate only under PCM', () => {
  daemon.set(reduceStatus(empty(), remoteAsking('wav', 44_100)));
  const wav = render(SpeakingPage, { voices: VOICES });
  expect(rateField()).toBeNull();
  wav.unmount();

  daemon.set(reduceStatus(empty(), remoteAsking('pcm', 22_050)));
  render(SpeakingPage, { voices: VOICES });
  expect(rateField()?.value).toBe('22050');
  expect(rateField()?.placeholder).toBe('24000');
});

it('writes the rate as a number', async () => {
  daemon.set(reduceStatus(empty(), remoteAsking('pcm', null)));
  render(SpeakingPage, { voices: VOICES });
  const field = rateField()!;
  await fireEvent.input(field, { target: { value: ' 22050 ' } });
  await fireEvent.blur(field);
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.remote.sample_rate', 22_050);
});

// An empty field goes back to the rate Banshee assumes, not to a rate of 0.
it('clears the rate when the field is emptied', async () => {
  daemon.set(reduceStatus(empty(), remoteAsking('pcm', 22_050)));
  render(SpeakingPage, { voices: VOICES });
  const field = rateField()!;
  await fireEvent.input(field, { target: { value: '' } });
  await fireEvent.blur(field);
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.remote.sample_rate', null);
});

it('sends nothing for a rate that is not a whole number, and says why', async () => {
  daemon.set(reduceStatus(empty(), remoteAsking('pcm', 22_050)));
  render(SpeakingPage, { voices: VOICES });
  const field = rateField()!;
  await fireEvent.input(field, { target: { value: '22.05k' } });
  await fireEvent.blur(field);
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
  await waitFor(() => expect(field.value).toBe('22050'));
  expect(get(announcement)).toBe('The sample rate is a whole number of hertz, as in 24000.');
});

it('says on the Sample rate row why a typed rate was not sent', async () => {
  daemon.set(reduceStatus(empty(), remoteAsking('pcm', 22_050)));
  render(SpeakingPage, { voices: VOICES });
  const field = rateField()!;
  await fireEvent.input(field, { target: { value: 'abc' } });
  await fireEvent.blur(field);

  const refusal = await waitFor(() =>
    screen.getByText('The sample rate is a whole number of hertz, as in 24000.'),
  );
  expect(refusal.closest('.row')?.contains(field)).toBe(true);
  expect(field.getAttribute('aria-describedby')).toBe(refusal.id);
  expect(field.value).toBe('22050');
});

it('orders the remote rows from the server down to the format', () => {
  daemon.set(reduceStatus(empty(), remoteAsking('pcm')));
  render(SpeakingPage, { voices: VOICES });
  const group = within(speakingGroup());
  const rows = [
    ['Server', group.getByRole('textbox', { name: 'Server' })],
    ['Key', group.getByText('A key is set')],
    ['Model', group.getByRole('textbox', { name: 'Model' })],
    ['Voice', group.getByRole('textbox', { name: 'Voice' })],
    ['Tone', group.getByRole('textbox', { name: 'Tone' })],
    ['Format', group.getByRole('radiogroup', { name: 'Audio format' })],
    ['Sample rate', group.getByRole('textbox', { name: 'Sample rate' })],
  ] as const;
  const order: HTMLElement[] = [screen.getByRole('radiogroup', { name: 'Speaking' })];
  for (const [name, control] of rows) {
    const label = group.getByText(name);
    expect(label.classList.contains('caps')).toBe(true);
    order.push(label, control);
  }
  expectInOrder(...order);
});

// The daemon refuses both, but its answer is a TOML parse error over four
// lines, and the page says it aloud.
it.each(['0', '4294967296'])('sends nothing for a rate of %s', async (typed) => {
  daemon.set(reduceStatus(empty(), remoteAsking('pcm', 22_050)));
  render(SpeakingPage, { voices: VOICES });
  const field = rateField()!;
  await fireEvent.input(field, { target: { value: typed } });
  await fireEvent.blur(field);
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
  await waitFor(() => expect(field.value).toBe('22050'));
  expect(get(announcement)).toBe('The sample rate is a whole number of hertz, as in 24000.');
});

it('puts the whole description in the row', () => {
  const { container } = render(SpeakingPage, {
    voices: {
      voices: [
        { id: 'af_bella', name: 'Bella', description: 'American, warm', downloaded: true },
        { id: 'bf_isabella', name: 'Isabella', description: 'British, warm', downloaded: true },
      ],
      current: 'af_bella',
    },
  });
  expect([...container.querySelectorAll('.desc')].map((n) => n.textContent)).toEqual([
    'American, warm',
    'British, warm',
  ]);
});

function voiceRadios(): string[] {
  return within(screen.getByRole('radiogroup', { name: 'Voice' }))
    .getAllByRole('radio')
    .map((radio) => radio.id);
}

function moreVoices(): HTMLElement {
  return screen.getByRole('list', { name: 'More voices' });
}

it('offers only voices on the machine as choices', () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  render(SpeakingPage, { voices: VOICES });
  expect(voiceRadios()).toEqual(['voice-af_sky', 'voice-af_heart']);
  expect(document.querySelectorAll('input[type="radio"][name="voice"]')).toHaveLength(2);
});

// jsdom moves no radio on an arrow key. A browser moves to the next radio of the
// same name in tree order, so that order is what decides where ArrowDown lands.
it('moves among voices on the machine with the arrow keys', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  render(SpeakingPage, { voices: VOICES });
  const named = [...document.querySelectorAll<HTMLInputElement>('input[name="voice"]')];
  const next = named[named.findIndex((radio) => radio.id === 'voice-af_sky') + 1];
  expect(next.id).toBe('voice-af_heart');

  await fireEvent.click(next);
  await waitFor(() => expect(vi.mocked(setSetting)).toHaveBeenCalledWith('tts.voice', 'af_heart'));
  expect(vi.mocked(downloadModels)).not.toHaveBeenCalled();
});

it('lists the rest under More voices with Get', () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  render(SpeakingPage, { voices: VOICES });
  const more = moreVoices();
  expect(more.tagName).toBe('UL');
  expect(
    within(more)
      .getAllByRole('button')
      .map((button) => button.getAttribute('aria-label')),
  ).toEqual([
    'Get Companion and switch to it',
    'Get Bella and switch to it',
    'Get Adam and switch to it',
  ]);
  expect(within(more).queryByRole('radio')).toBeNull();
  expect(more.querySelector('.absent')).toBeNull();
  const note = screen.getByText('Get downloads a voice and switches to it.');
  expect(note.closest('.row')?.contains(more)).toBe(true);
  expect(more.getAttribute('aria-describedby')).toBe(note.id);
});

it('keeps a Get on the current voice while its file is absent', () => {
  daemon.set(reduceStatus(empty(), speaking({ voice: 'af_bella' })));
  render(SpeakingPage, { voices: VOICES });
  expect(voiceRadios()).toEqual(['voice-af_sky', 'voice-af_bella', 'voice-af_heart']);
  const bella = screen.getByRole('radio', { name: /Bella.*not on this machine/ });
  expect((bella as HTMLInputElement).checked).toBe(true);
  expect(document.querySelector('label[for="voice-af_bella"] .name')?.classList).toContain(
    'absent',
  );
  const group = screen.getByRole('radiogroup', { name: 'Voice' });
  expect(
    within(group).getByRole('button', { name: 'Get Bella, which is not on this machine' }),
  ).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Play Bella' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Get Bella and switch to it' })).toBeNull();
});

it('shows the download under the current voice while its file arrives', () => {
  daemon.set({
    ...reduceStatus(empty(), { ...speaking({ voice: 'af_bella' }), pending: ['tts.voice'] }),
    download: {
      model: 'af_bella.bin',
      label: 'Voice',
      index: 1,
      count: 1,
      bytes: 5_000_000,
      total: 10_000_000,
      state: 'downloading',
    },
  });
  const { container } = render(SpeakingPage, { voices: VOICES });
  const line = container.querySelector('.progress') as HTMLElement;
  expect(line.textContent).toContain('50%');
  expect(line.closest('.row')).toBe(
    screen.getByRole('radiogroup', { name: 'Voice' }).closest('.row'),
  );
  expect(container.querySelector('.note.pending')).toBeNull();
});

it('lands focus on the voice when Get chooses it', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  vi.mocked(status).mockResolvedValue(speaking({ voice: 'af_bella' }));
  render(SpeakingPage, { voices: VOICES });

  await fireEvent.click(screen.getByRole('button', { name: 'Get Bella and switch to it' }));

  await waitFor(() => expect(document.activeElement?.id).toBe('voice-af_bella'));
  expect(get(announcement)).toBe('Getting Bella.');
  expect(within(moreVoices()).queryByRole('button', { name: /Bella/ })).toBeNull();
  await waitFor(() => expect(vi.mocked(downloadModels)).toHaveBeenCalled());
});

it('moves the voice Get chose into Voice, and nothing when its file lands', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  vi.mocked(status).mockResolvedValue(speaking({ voice: 'af_bella' }));
  const page = render(SpeakingPage, { voices: VOICES });
  expect(travel).not.toHaveBeenCalled();

  await fireEvent.click(screen.getByRole('button', { name: 'Get Bella and switch to it' }));
  await waitFor(() => expect(document.activeElement?.id).toBe('voice-af_bella'));
  expect(vi.mocked(travel).mock.calls).toEqual([['af_bella']]);

  await page.rerender({
    voices: {
      ...VOICES,
      voices: VOICES.voices.map((voice) =>
        voice.id === 'af_bella' ? { ...voice, downloaded: true } : voice,
      ),
    },
  });
  expect(vi.mocked(travel).mock.calls).toEqual([['af_bella']]);
});

it('says when a fetched voice is ready', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  vi.mocked(status).mockResolvedValue(speaking({ voice: 'af_bella' }));
  const page = render(SpeakingPage, { voices: VOICES });
  await fireEvent.click(screen.getByRole('button', { name: 'Get Bella and switch to it' }));
  await waitFor(() => expect(vi.mocked(downloadModels)).toHaveBeenCalled());

  const landed: Voices = {
    ...VOICES,
    voices: VOICES.voices.map((voice) =>
      voice.id === 'af_bella' ? { ...voice, downloaded: true } : voice,
    ),
  };
  await page.rerender({ voices: landed });
  expect(get(announcement)).toBe('Bella is ready.');

  forgetCopy();
  await page.rerender({ voices: { ...landed, voices: [...landed.voices] } });
  expect(get(announcement)).toBe('');
});

it('says which voice did not download, and keeps its Get to try again', async () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  vi.mocked(status).mockResolvedValue(speaking({ voice: 'am_adam' }));
  vi.mocked(downloadModels).mockRejectedValue(new Error('offline'));
  render(SpeakingPage, { voices: VOICES });
  await fireEvent.click(screen.getByRole('button', { name: 'Get Adam and switch to it' }));
  await waitFor(() => expect(get(problem)).toBe('Adam did not download. Press Get to try again.'));

  const retry = within(screen.getByRole('radiogroup', { name: 'Voice' })).getByRole('button', {
    name: 'Get Adam, which is not on this machine',
  });
  vi.mocked(downloadModels).mockResolvedValue(undefined);
  await fireEvent.click(retry);
  await waitFor(() => expect(vi.mocked(downloadModels)).toHaveBeenCalledTimes(2));
});

it('leaves More voices out when every voice is here', () => {
  daemon.set(reduceStatus(empty(), LOCAL));
  render(SpeakingPage, {
    voices: {
      voices: VOICES.voices.map((voice) => ({ ...voice, downloaded: true })),
      current: 'af_sky',
    },
  });
  expect(screen.queryByRole('list', { name: 'More voices' })).toBeNull();
  expect(screen.queryByText('More voices')).toBeNull();
});

it('says it cannot list voices while Banshee is not running', () => {
  daemon.set(reduceStatus(empty(), { ...LOCAL, running: false }));
  const down = render(SpeakingPage, { voices: { voices: [], current: null } });
  expect(screen.getByText('Banshee is not running, so it cannot list its voices.')).toBeTruthy();
  down.unmount();

  daemon.set(reduceStatus(empty(), LOCAL));
  render(SpeakingPage, { voices: { voices: [], current: null } });
  expect(screen.getByText("No voices yet. They arrive with Banshee's models.")).toBeTruthy();
});

const BUSY = 'One download at a time. This works again when the current one finishes.';

const RUNNING: Progress = {
  model: 'kokoro.onnx',
  label: 'Voice',
  index: 1,
  count: 2,
  bytes: 1,
  total: 10,
  state: 'downloading',
};

it('keeps the Get note standing while a download runs, and leaves Get to press', () => {
  daemon.set({ ...reduceStatus(empty(), LOCAL), download: RUNNING });
  const { container } = render(SpeakingPage, { voices: VOICES });
  expect(screen.queryByText(BUSY)).toBeNull();
  expect(container.querySelector('.progress')).toBeNull();
  const note = screen.getByText('Get downloads a voice and switches to it.');
  expect(note.closest('.row')?.contains(moreVoices())).toBe(true);
  const button = screen.getByRole('button', { name: 'Get Adam and switch to it' });
  expect((button as HTMLButtonElement).disabled).toBe(false);
});

it('marks every Get busy while a download runs, and clears the mark when it ends', async () => {
  daemon.set({ ...reduceStatus(empty(), LOCAL), download: RUNNING });
  render(SpeakingPage, { voices: VOICES });
  const gets = within(moreVoices()).getAllByRole('button');
  expect(gets).toHaveLength(3);
  for (const button of gets) {
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect((button as HTMLButtonElement).disabled).toBe(false);
  }

  daemon.set(reduceStatus(empty(), LOCAL));
  await tick();

  for (const button of within(moreVoices()).getAllByRole('button')) {
    expect(button.hasAttribute('aria-disabled')).toBe(false);
  }
});

it('says once, under the Get pressed, that one download runs at a time', async () => {
  daemon.set({ ...reduceStatus(empty(), LOCAL), download: RUNNING });
  render(SpeakingPage, { voices: VOICES });
  const said: string[] = [];
  const stop = announcement.subscribe((message) => said.push(message));

  const adam = screen.getByRole('button', { name: 'Get Adam and switch to it' });
  await fireEvent.click(adam);

  stop();
  expect(said.filter((message) => message === BUSY)).toHaveLength(1);
  expect(vi.mocked(setSetting)).not.toHaveBeenCalled();
  const busy = screen.getByText(BUSY);
  expect(busy.closest('li')).toBe(adam.closest('li'));
  expect(adam.getAttribute('aria-describedby')?.split(' ')).toContain(busy.id);
});

it('moves the busy sentence to the next Get pressed, and clears it when the run ends', async () => {
  daemon.set({ ...reduceStatus(empty(), LOCAL), download: RUNNING });
  render(SpeakingPage, { voices: VOICES });
  const adam = screen.getByRole('button', { name: 'Get Adam and switch to it' });
  const bella = screen.getByRole('button', { name: 'Get Bella and switch to it' });
  await fireEvent.click(adam);
  await fireEvent.click(bella);

  expect(screen.getAllByText(BUSY)).toHaveLength(1);
  expect(screen.getByText(BUSY).closest('li')).toBe(bella.closest('li'));
  expect(adam.getAttribute('aria-describedby')).not.toContain(screen.getByText(BUSY).id);

  daemon.set(reduceStatus(empty(), LOCAL));
  await tick();

  expect(screen.queryByText(BUSY)).toBeNull();
  expect(bella.getAttribute('aria-describedby')).toBe(
    screen.getByText('Get downloads a voice and switches to it.').id,
  );
});

it('keeps focus on the button beside the current voice when its file lands', async () => {
  daemon.set(reduceStatus(empty(), speaking({ voice: 'af_bella' })));
  const page = render(SpeakingPage, { voices: VOICES });
  const button = within(screen.getByRole('radiogroup', { name: 'Voice' })).getByRole('button', {
    name: 'Get Bella, which is not on this machine',
  });
  button.focus();

  await page.rerender({
    voices: {
      ...VOICES,
      voices: VOICES.voices.map((voice) =>
        voice.id === 'af_bella' ? { ...voice, downloaded: true } : voice,
      ),
    },
  });

  expect(document.activeElement).toBe(button);
  expect(button.getAttribute('aria-label')).toBe('Play Bella');
  expect(button.textContent?.trim()).toBe('Play');
});

it('offers a voice that is not here the control that gets it', () => {
  render(SpeakingPage, { voices: VOICES });

  expect(screen.getByRole('button', { name: 'Get Adam and switch to it' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Play Adam' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Play Sky' })).toBeTruthy();
});

it('sends one fetch however fast Get is pressed', async () => {
  vi.mocked(downloadModels).mockResolvedValue(undefined);
  render(SpeakingPage, { voices: VOICES });

  const get = screen.getByRole('button', { name: 'Get Adam and switch to it' });
  get.click();
  get.click();
  screen.getByRole('button', { name: 'Get Bella and switch to it' }).click();

  await waitFor(() => expect(vi.mocked(downloadModels)).toHaveBeenCalledTimes(1));
  expect(vi.mocked(setSetting)).toHaveBeenCalledTimes(1);
  expect((get as HTMLButtonElement).disabled).toBe(false);
});

it("lists the companion first, with Kyutai's credit", () => {
  render(SpeakingPage, { voices: VOICES });
  const first = within(moreVoices()).getAllByRole('listitem')[0];
  expect(within(first).getByText('Companion')).toBeTruthy();
  expect(within(first).getByText('Pocket TTS by Kyutai, CC-BY-4.0')).toBeTruthy();
});

it('has no speaking rate', () => {
  render(SpeakingPage, { voices: VOICES });
  expect(screen.queryByText('Speaking rate')).toBeNull();
  expect(screen.queryByRole('slider')).toBeNull();
});

it('offers Get Companion when its files have not arrived', () => {
  render(SpeakingPage, { voices: VOICES });
  expect(screen.getByRole('button', { name: 'Get Companion and switch to it' })).toBeTruthy();
  expect(screen.queryByRole('radio', { name: /Companion/ })).toBeNull();
});
