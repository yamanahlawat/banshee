import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('./tauri', () => ({ setSetting: vi.fn(), status: vi.fn() }));
import { setSetting, status } from './tauri';
import { daemon, empty } from './daemon';
import { forgetRefusal, forgetRefusals, refuse, refused, set, write } from './settings';

beforeEach(() => {
  vi.clearAllMocks();
  daemon.set(empty());
  forgetRefusals();
  vi.mocked(status).mockResolvedValue({ running: true, config: {} });
});

it('marks a setting pending when the daemon says it could not use it yet', async () => {
  vi.mocked(setSetting).mockResolvedValue(['feedback.mode']);
  // The daemon keeps the pending set itself, and its status carries it.
  vi.mocked(status).mockResolvedValue({ running: true, pending: ['feedback.mode'] });
  await set('feedback.mode', 'none');
  expect(get(daemon).pending.has('feedback.mode')).toBe(true);
});

it('drops a mark the daemon no longer reports', async () => {
  vi.mocked(setSetting).mockResolvedValue(['feedback.mode']);
  vi.mocked(status).mockResolvedValue({ running: true, pending: [] });
  await set('feedback.mode', 'none');
  expect(get(daemon).pending.size).toBe(0);
});

it('marks nothing pending when the daemon applied it live', async () => {
  vi.mocked(setSetting).mockResolvedValue([]);
  await set('audio.input_device', 'MacBook Pro Microphone');
  expect(get(daemon).pending.size).toBe(0);
});

it('sends the key and value the caller named', async () => {
  vi.mocked(setSetting).mockResolvedValue([]);
  await set('stt.vad_threshold', 0.5);
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.vad_threshold', 0.5);
});

it('reads the daemon again, so a row shows what was just written', async () => {
  vi.mocked(setSetting).mockResolvedValue([]);
  vi.mocked(status).mockResolvedValue({ running: true, config: { audio: { hotkey: 'F6' } } });
  await set('audio.hotkey', 'F6');
  expect(get(daemon).status?.config?.audio).toEqual({ hotkey: 'F6' });
});

it('keeps the write when the re-read fails', async () => {
  vi.mocked(setSetting).mockResolvedValue(['audio.hotkey']);
  vi.mocked(status).mockRejectedValue(new Error('gone'));
  await set('audio.hotkey', 'F6');
  expect(get(daemon).pending.has('audio.hotkey')).toBe(true);
});

it('reports that a write the daemon took has landed', async () => {
  vi.mocked(setSetting).mockResolvedValue([]);
  expect(await write('tts.voice', 'af_sky', () => {})).toBe(true);
});

it('reports that a refused write has not landed', async () => {
  vi.mocked(setSetting).mockRejectedValue(new Error('no such voice'));
  const said: string[] = [];
  expect(await write('tts.voice', 'af_sky', (m) => said.push(m))).toBe(false);
  expect(said).toEqual(['Not changed. No such voice.']);
});

const STOPPED = { code: -32000, message: 'the daemon closed the socket', transport: true };

it('keeps a refusal under its key and says the same sentence once', async () => {
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  const said: string[] = [];
  await write('feedback.mode', 'none', (m) => said.push(m));
  expect(get(refused).get('feedback.mode')).toBe('Not changed. Banshee is not running.');
  expect(said).toEqual(['Not changed. Banshee is not running.']);
});

it('drops a refusal as soon as its key is written again', async () => {
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  await write('feedback.mode', 'none', () => {});
  vi.mocked(setSetting).mockReturnValue(new Promise(() => {}));
  void write('feedback.mode', 'sound', () => {});
  expect(get(refused).has('feedback.mode')).toBe(false);
});

// The socket answers in order, so a refusal of the first of two quick writes
// arrives after the second one has started.
it('drops a refusal that arrives while a later write of its key is under way', async () => {
  let refuse: (error: unknown) => void = () => {};
  let land: (waiting: string[]) => void = () => {};
  vi.mocked(setSetting)
    .mockReturnValueOnce(new Promise((_, reject) => (refuse = reject)))
    .mockReturnValueOnce(new Promise((resolve) => (land = resolve)));
  const first = write('feedback.mode', 'none', () => {});
  const second = write('feedback.mode', 'sound', () => {});
  refuse(STOPPED);
  await first;
  land([]);
  await second;
  expect(get(refused).has('feedback.mode')).toBe(false);
});

it('keeps the refusal of a key that is not written again', async () => {
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  await write('feedback.mode', 'none', () => {});
  vi.mocked(setSetting).mockResolvedValue([]);
  await write('audio.barge_in', 'stop', () => {});
  expect(get(refused).has('feedback.mode')).toBe(true);
});

it('forgets every refusal at once', async () => {
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  await write('feedback.mode', 'none', () => {});
  await write('audio.barge_in', 'stop', () => {});
  forgetRefusals();
  expect(get(refused).size).toBe(0);
});

// A page change forgets every refusal, so one that lands after it belongs to a
// row nobody is looking at, and would stand there when the page comes back.
it('says a refusal that lands after the refusals were forgotten, and keeps none', async () => {
  let fail: (error: unknown) => void = () => {};
  vi.mocked(setSetting).mockReturnValueOnce(new Promise((_, reject) => (fail = reject)));
  const said: string[] = [];
  const pending = write('feedback.mode', 'none', (m) => said.push(m));
  forgetRefusals();
  fail(STOPPED);
  expect(await pending).toBe(false);
  expect(get(refused).has('feedback.mode')).toBe(false);
  expect(said).toEqual(['Not changed. Banshee is not running.']);
});

it('keeps a refusal from a write that began after the refusals were forgotten', async () => {
  forgetRefusals();
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  await write('feedback.mode', 'none', () => {});
  expect(get(refused).get('feedback.mode')).toBe('Not changed. Banshee is not running.');
});

it('stores a refusal and says it once', () => {
  const said: string[] = [];
  refuse('audio.hotkey', 'Banshee cannot bind that key.', (m) => said.push(m));
  expect(get(refused).get('audio.hotkey')).toBe('Banshee cannot bind that key.');
  expect(said).toEqual(['Banshee cannot bind that key.']);
});

it('neither keeps nor says a refusal once a later write of its key has begun', async () => {
  let fail: (error: unknown) => void = () => {};
  vi.mocked(setSetting)
    .mockReturnValueOnce(new Promise((_, reject) => (fail = reject)))
    .mockReturnValueOnce(new Promise(() => {}));
  const said: string[] = [];
  const first = write('feedback.mode', 'none', (m) => said.push(m));
  void write('feedback.mode', 'sound', (m) => said.push(m));
  fail(STOPPED);
  expect(await first).toBe(false);
  expect(get(refused).has('feedback.mode')).toBe(false);
  expect(said).toEqual([]);
});

// Svelte notifies on every set of an object, so a forget with nothing to drop
// would re-render every row.
it('tells no subscriber when there is nothing to forget', () => {
  refuse('audio.hotkey', 'Banshee cannot bind that key.', () => {});
  let told = 0;
  const stop = refused.subscribe(() => (told += 1));
  forgetRefusal('feedback.mode');
  expect(told).toBe(1);
  forgetRefusal('audio.hotkey');
  forgetRefusal('audio.hotkey');
  forgetRefusals();
  expect(told).toBe(2);
  stop();
});
