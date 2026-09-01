import { get } from 'svelte/store';
import { beforeEach, expect, it, vi } from 'vitest';
vi.mock('./tauri', () => ({ setSetting: vi.fn(), status: vi.fn() }));
import { setSetting, status } from './tauri';
import { daemon, empty } from './daemon';
import { set, write } from './settings';

beforeEach(() => {
  vi.clearAllMocks();
  daemon.set(empty());
  vi.mocked(status).mockResolvedValue({ running: true, config: {} });
});

it('marks a setting pending when the daemon says it needs a restart', async () => {
  vi.mocked(setSetting).mockResolvedValue(['audio.cues.enabled']);
  // The daemon keeps the pending set itself, and its status carries it.
  vi.mocked(status).mockResolvedValue({ running: true, pending: ['audio.cues.enabled'] });
  await set('audio.cues.enabled', true);
  expect(get(daemon).pending.has('audio.cues.enabled')).toBe(true);
});

it('drops a mark the daemon no longer reports', async () => {
  vi.mocked(setSetting).mockResolvedValue(['audio.cues.enabled']);
  vi.mocked(status).mockResolvedValue({ running: true, pending: [] });
  await set('audio.cues.enabled', true);
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
  expect(said).toEqual(['no such voice']);
});
