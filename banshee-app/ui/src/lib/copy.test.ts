import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('./tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));
import {
  announcement,
  copy,
  copied,
  listeningLead,
  listeningNote,
  problem,
  speechLead,
  speechNote,
  TAKES_EFFECT,
} from './copy';
import type { Listening, Speech } from './daemon';
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

// A remote side in force, with a different host in the config, so a sentence
// that reads the one it was not given is caught.
const HEARD: Listening = {
  live: true,
  remote: true,
  keyPresent: true,
  pending: false,
  host: 'api.openai.com',
  willUse: 'api.groq.com',
  device: 'MacBook Pro Microphone',
  pipelineBroken: false,
};

const SPOKEN: Speech = {
  live: true,
  remote: true,
  started: true,
  keyPresent: true,
  pending: false,
  host: 'api.openai.com',
  willUse: 'api.groq.com',
  voiceName: 'marin',
};

const LISTENING_LEADS: [string, Partial<Listening>, string][] = [
  [
    'a stopped daemon has no microphone open',
    { live: false, remote: false, pending: true, device: null, pipelineBroken: true },
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
    'Banshee sends what you say to a remote server to be heard.',
  ],
  [
    'a flip away from a remote listener still sends there',
    { pending: true },
    'Banshee sends what you say to api.openai.com until it restarts.',
  ],
  [
    'a flip to a remote listener names the server the config asks for',
    { remote: false, pending: true },
    'Banshee will send what you say to api.groq.com when it restarts.',
  ],
  [
    'a flip to a server the config does not name',
    { remote: false, pending: true, willUse: null },
    'Banshee will send what you say to a remote server when it restarts.',
  ],
  [
    'a local listener names its device',
    { remote: false },
    'Banshee is listening through the MacBook Pro Microphone.',
  ],
  [
    'a device that opened outranks a broken pipeline',
    { remote: false, pipelineBroken: true },
    'Banshee is listening through the MacBook Pro Microphone.',
  ],
  [
    'a broken pipeline with no device',
    { remote: false, device: null, pipelineBroken: true },
    'Banshee cannot open a microphone.',
  ],
  [
    'no device and nothing broken',
    { remote: false, device: null },
    'Banshee is not listening yet.',
  ],
];

it.each(LISTENING_LEADS)('the microphone lead, when %s', (_, over, says) => {
  expect(listeningLead({ ...HEARD, ...over })).toBe(says);
});

const LISTENING_NOTES: [string, Partial<Listening>, string][] = [
  ['the daemon is down', { live: false, pending: true }, 'Nothing is heard until Banshee starts.'],
  ['a remote listener is in force', {}, 'Audio goes to api.openai.com.'],
  ['the daemon cannot name the host', { host: null }, 'Audio goes to a remote server.'],
  [
    'a flip away from a remote listener waits',
    { pending: true },
    `Audio still goes to api.openai.com. ${TAKES_EFFECT}`,
  ],
  [
    'a flip to a remote listener waits',
    { remote: false, pending: true },
    `Audio still stays on this machine. ${TAKES_EFFECT}`,
  ],
  [
    'a waiting flip says nothing about the key',
    { remote: false, pending: true, keyPresent: false },
    `Audio still stays on this machine. ${TAKES_EFFECT}`,
  ],
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
  [
    'the note reads no device of its own',
    { device: null, pipelineBroken: true },
    'Audio goes to api.openai.com.',
  ],
];

it.each(LISTENING_NOTES)('the listening note, when %s', (_, over, says) => {
  expect(listeningNote({ ...HEARD, ...over })).toBe(says);
});

const SPEECH_LEADS: [string, Partial<Speech>, string][] = [
  [
    'a stopped daemon speaks nothing',
    { live: false, started: false, pending: true },
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
    'Banshee speaks through a remote server as marin.',
  ],
  [
    'a flip away from a started speaker waits',
    { pending: true },
    'Banshee speaks through api.openai.com until it restarts.',
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
  [
    'a waiting flip outranks the fields that would fix the speaker',
    { started: false, pending: true, keyPresent: false, voiceName: '' },
    'Banshee cannot speak through api.openai.com. Your choice takes effect when it restarts.',
  ],
  ['a local speaker names its voice', { remote: false }, 'Banshee speaks as marin.'],
  ['a local speaker has no voice', { remote: false, voiceName: '' }, 'Banshee has no voice yet.'],
  [
    'a flip to a remote speaker names the server the config asks for',
    { remote: false, pending: true },
    'Banshee will speak through api.groq.com when it restarts.',
  ],
  [
    'a flip to a server the config does not name',
    { remote: false, pending: true, willUse: null },
    'Banshee will speak through a remote server when it restarts.',
  ],
];

it.each(SPEECH_LEADS)('the voice lead, when %s', (_, over, says) => {
  expect(speechLead({ ...SPOKEN, ...over })).toBe(says);
});

const SPEECH_NOTES: [string, Partial<Speech>, string][] = [
  ['the daemon is down', { live: false, pending: true }, 'Nothing is spoken until Banshee starts.'],
  ['a started remote speaker', {}, 'Text goes to api.openai.com.'],
  ['the daemon cannot name the host', { host: null }, 'Text goes to a remote server.'],
  [
    'a flip away from a remote speaker waits',
    { pending: true },
    `Text still goes to api.openai.com. ${TAKES_EFFECT}`,
  ],
  [
    'the speaker did not start',
    { started: false },
    'The speaker on api.openai.com did not start, so text stays on this machine.',
  ],
  [
    'the speaker did not start and a flip waits',
    { started: false, pending: true },
    `The speaker on api.openai.com did not start, so text stays on this machine. ${TAKES_EFFECT}`,
  ],
  [
    'a flip to a remote speaker waits',
    { remote: false, pending: true },
    `Text still stays on this machine. ${TAKES_EFFECT}`,
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
