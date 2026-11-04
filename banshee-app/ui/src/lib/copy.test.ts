import { get } from 'svelte/store';
import { beforeEach, describe, expect, it, vi } from 'vitest';
vi.mock('./tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));
import {
  announcement,
  CHOICE_LABELS,
  copy,
  copied,
  listeningLead,
  listeningNote,
  problem,
  speechLead,
  speechNote,
  languagesHeard,
  modelCost,
  modelFacts,
} from './copy';
import * as copyModule from './copy';
import { empty, listeningFacts, type Listening, type Speech } from './daemon';
import type { Setting } from './tauri';
import mockTable from '../mocks/settings.json';
beforeEach(() => vi.useFakeTimers());
it('marks the control Copied for a moment, then clears', async () => {
  await copy('hello', 'row-14:02');
  expect(get(copied)).toBe('row-14:02');
  vi.advanceTimersByTime(1500);
  expect(get(copied)).toBeNull();
});

it('clears the announcement, so a second copy is a change the region speaks', async () => {
  await copy('hello', 'row-14:02');
  expect(get(announcement)).toBe('Copied');
  vi.advanceTimersByTime(1500);
  expect(get(announcement)).toBe('');
});

it('says so when the clipboard refuses, rather than looking like it worked', async () => {
  const { copyText } = await import('./tauri');
  vi.mocked(copyText).mockRejectedValueOnce(new Error('clipboard unavailable'));

  await copy('hello', 'row-14:02');

  expect(get(copied)).toBeNull();
  expect(get(problem)).toMatch(/Nothing was copied/);
});

const HEARD: Listening = {
  live: true,
  remote: true,
  keyPresent: true,
  host: 'api.openai.com',
  device: 'MacBook Pro Microphone',
  open: true,
  keepOpen: false,
  loading: false,
  refused: false,
  stoppedBy: null,
};

const SPOKEN: Speech = {
  live: true,
  remote: true,
  started: true,
  keyPresent: true,
  host: 'api.openai.com',
  voiceName: 'marin',
  voiceDownloaded: true,
};

const LISTENING_LEADS: [string, Partial<Listening>, string][] = [
  [
    'a stopped daemon has no microphone open',
    { live: false, remote: false, device: null, stoppedBy: 'pipeline' },
    'Banshee is not running, so no microphone is open.',
  ],
  [
    'a remote listener names its host',
    {},
    'Banshee sends what you say to api.openai.com to be heard.',
  ],
  [
    'a host the daemon cannot name',
    { host: null },
    'Banshee sends what you say to a transcription server to be heard.',
  ],
  [
    'a local listener names the device it is listening through',
    { remote: false },
    'Banshee is listening through the MacBook Pro Microphone.',
  ],
  [
    'an idle microphone names the device the next use opens',
    { remote: false, open: false },
    'Banshee opens the MacBook Pro Microphone when you speak.',
  ],
  [
    'keep_open holds the microphone open between uses',
    { remote: false, keepOpen: true },
    'Banshee keeps the MacBook Pro Microphone open.',
  ],
  [
    'the listening model is loading',
    { remote: false, open: false, loading: true },
    'Banshee is loading the listening model.',
  ],
  [
    'macOS refuses the microphone before any press',
    { remote: false, open: false, refused: true },
    'macOS does not allow Banshee to use the microphone.',
  ],
  [
    'a device that opened outranks a broken pipeline',
    { remote: false, stoppedBy: 'pipeline' },
    'Banshee is listening through the MacBook Pro Microphone.',
  ],
  [
    'a broken pipeline with no device',
    { remote: false, device: null, stoppedBy: 'pipeline' },
    'Banshee cannot open a microphone.',
  ],
  [
    'no device and nothing broken',
    { remote: false, device: null },
    'Banshee is not listening yet.',
  ],
  [
    'a remote listener names the detector it loads, never a listening model',
    { loading: true },
    'Banshee is loading the voice detection model.',
  ],
  [
    'a remote listener that never started',
    { stoppedBy: 'provider' },
    'Banshee cannot reach api.openai.com to hear you.',
  ],
  [
    'a remote listener whose key file cannot be read',
    { stoppedBy: 'keyfile' },
    'Banshee cannot read the key file for api.openai.com.',
  ],
];

it.each(LISTENING_LEADS)('the microphone lead, when %s', (_, over, says) => {
  expect(listeningLead({ ...HEARD, ...over })).toBe(says);
});

const LISTENING_NOTES: [string, Partial<Listening>, string][] = [
  ['the daemon is down', { live: false }, 'Nothing is heard until Banshee starts.'],
  ['a remote listener is in force', {}, 'Audio goes to api.openai.com.'],
  ['the daemon cannot name the host', { host: null }, 'Audio goes to a transcription server.'],
  [
    'a local listener still holds a key',
    { remote: false },
    'Audio stays on this machine. The server and key you set are still saved.',
  ],
  [
    'a local listener holds no key',
    { remote: false, keyPresent: false },
    'Audio stays on this machine.',
  ],
  ['the note reads no device of its own', { device: null }, 'Audio goes to api.openai.com.'],
  [
    'a remote listener that never started',
    { stoppedBy: 'provider', host: 'api.openai.com' },
    'Nothing goes to api.openai.com until the listener starts.',
  ],
  [
    'a remote listener whose key file cannot be read',
    { stoppedBy: 'keyfile' },
    'Nothing goes to api.openai.com until the key file is removed and the key is set again.',
  ],
];

it.each(LISTENING_NOTES)('the listening note, when %s', (_, over, says) => {
  expect(listeningNote({ ...HEARD, ...over })).toBe(says);
});

const SPEECH_LEADS: [string, Partial<Speech>, string][] = [
  [
    'a stopped daemon speaks nothing',
    { live: false, started: false },
    'Banshee is not running, so nothing is spoken.',
  ],
  [
    'a remote speaker names its host and voice',
    {},
    'Banshee speaks through api.openai.com as marin.',
  ],
  ['no voice is named', { voiceName: '' }, 'Banshee speaks through api.openai.com.'],
  [
    'the daemon cannot name the host',
    { host: null },
    'Banshee speaks through a voice server as marin.',
  ],
  ['the speaker did not start', { started: false }, 'Banshee cannot speak through api.openai.com.'],
  [
    'the speaker did not start and holds no key',
    { started: false, keyPresent: false },
    'Banshee cannot speak through api.openai.com until you paste a key.',
  ],
  [
    'the speaker did not start and names no voice',
    { started: false, voiceName: '' },
    'Banshee cannot speak through api.openai.com until you name a voice.',
  ],
  [
    'the missing key is named before the missing voice',
    { started: false, keyPresent: false, voiceName: '' },
    'Banshee cannot speak through api.openai.com until you paste a key.',
  ],
  ['a local speaker names its voice', { remote: false }, 'Banshee speaks as marin.'],
  ['a local speaker has no voice', { remote: false, voiceName: '' }, 'Banshee has no voice yet.'],
  [
    'the local voice is not on this machine',
    { remote: false, started: false, voiceName: 'Bella', voiceDownloaded: false },
    'Bella did not load, so Banshee is silent. Get its files below, or pick another voice.',
  ],
  [
    'the local voice is on this machine and did not load',
    { remote: false, started: false, voiceName: 'Bella' },
    'Bella did not load, so Banshee is silent. Pick another voice.',
  ],
];

it.each(SPEECH_LEADS)('the voice lead, when %s', (_, over, says) => {
  expect(speechLead({ ...SPOKEN, ...over })).toBe(says);
});

const SPEECH_NOTES: [string, Partial<Speech>, string][] = [
  ['the daemon is down', { live: false }, 'Nothing is spoken until Banshee starts.'],
  ['a started remote speaker', {}, 'Text goes to api.openai.com.'],
  ['the daemon cannot name the host', { host: null }, 'Text goes to a voice server.'],
  [
    'the speaker did not start',
    { started: false },
    'The speaker on api.openai.com did not start, so text stays on this machine.',
  ],
  [
    'a local speaker still holds a key',
    { remote: false },
    'Text stays on this machine. The server and key you set are still saved.',
  ],
  [
    'a speaker that never started is silent under a local one',
    { remote: false, started: false },
    'Text stays on this machine. The server and key you set are still saved.',
  ],
  [
    'a local speaker holds no key',
    { remote: false, keyPresent: false },
    'Text stays on this machine.',
  ],
];

it.each(SPEECH_NOTES)('the speaking note, when %s', (_, over, says) => {
  expect(speechNote({ ...SPOKEN, ...over })).toBe(says);
});

describe('languagesHeard', () => {
  it('says English for a model that hears English alone', () => {
    expect(languagesHeard(['en'])).toBe('English');
  });

  // One language that is not English is still one language, never "many".
  it('names a single other language', () => {
    expect(languagesHeard(['ru'])).toBe('Russian');
  });

  it('names up to three languages', () => {
    expect(languagesHeard(['zh', 'en'])).toBe('Chinese and English');
    expect(languagesHeard(['zh', 'en', 'ja'])).toBe('Chinese, English, and Japanese');
  });

  it('counts the languages beyond three', () => {
    expect(languagesHeard(['en', 'de', 'es', 'fr'])).toBe('4 languages');
  });
});

describe('modelFacts', () => {
  it('states the size, the languages and, on macOS, whether words show as you speak', () => {
    expect(modelFacts({ megabytes: 731, languages: ['en'], streams: true }, true)).toBe(
      '731 MB. English. Shows words as you speak.',
    );
    expect(modelFacts({ megabytes: 1667, languages: ['zh', 'en'], streams: false }, true)).toBe(
      '1.7 GB. Chinese and English. Shows words once you stop.',
    );
  });

  // Only the macOS chip shows words as they are heard.
  it('says nothing about live words off macOS', () => {
    expect(modelFacts({ megabytes: 731, languages: ['en'], streams: true }, false)).toBe(
      '731 MB. English.',
    );
  });
});

describe('modelCost', () => {
  it('prices the file when it is the only thing missing', () => {
    expect(modelCost('Whisper turbo', 1031, 1031)).toBe(
      'Whisper turbo is not on this machine. About 1.0 GB to fetch.',
    );
  });

  // Download starts a run over everything absent, so the file's own size
  // understates the press whenever the detector or a voice is missing too.
  it('names the whole run when pressing brings more than the model', () => {
    const said = modelCost('Whisper turbo', 1031, 1345);
    expect(said).toContain('About 1.0 GB to fetch');
    expect(said).toContain('1.3 GB in all');
  });
});

// No panel shows tts.fallback: the CLI sets it.
it('names every choice a panel offers', () => {
  for (const row of mockTable as Setting[]) {
    if (row.domain.kind !== 'choice' || row.key === 'tts.fallback') continue;
    for (const value of row.domain.values) {
      expect(CHOICE_LABELS[row.key]?.[value], `${row.key} = ${value}`).toBeTruthy();
    }
  }
});

it('says nothing about a restart', () => {
  for (const said of Object.values(copyModule).filter((one) => typeof one === 'string')) {
    expect(said).not.toMatch(/restart/i);
  }
  for (const remote of [true, false]) {
    for (const keyPresent of [true, false]) {
      for (const started of [true, false]) {
        const side = {
          ...listeningFacts(empty()),
          live: true,
          remote,
          keyPresent,
          host: 'api.example.test',
        };
        const speech = { ...side, started, voiceName: 'alloy', voiceDownloaded: true };
        for (const said of [
          listeningLead(side),
          listeningNote(side),
          speechLead(speech),
          speechNote(speech),
        ]) {
          expect(said).not.toMatch(/restart/i);
        }
      }
    }
  }
});
