import { beforeEach, expect, it } from 'vitest';
import { answer } from './bridge';
import type { Status } from '../lib/daemon';

// A `?state=` is the only way to look at a state, so a state nobody can select
// is a state nobody reviews. These assert the shape each one answers with, not
// what the window then draws.
function at(state: string): Promise<Status> {
  window.history.replaceState({}, '', `/?state=${state}`);
  return answer<Status>('status');
}

function tts(status: Status): Record<string, unknown> {
  return (status.config?.tts ?? {}) as Record<string, unknown>;
}

beforeEach(() => {
  window.history.replaceState({}, '', '/');
});

// The speaker the config asks for and the one the daemon runs, in each
// direction. The heading reads both, so both directions need a mock.
it('serves a speaker on its way to a remote server', async () => {
  const status = await at('to-remote-voice');
  expect(tts(status).provider).toBe('remote');
  expect(status.remote?.tts.remote).toBe(false);
  expect(status.pending).toContain('tts.provider');
});

it('serves a speaker on its way back to this machine', async () => {
  const status = await at('to-local-voice');
  expect(tts(status).provider).toBe('local');
  expect(status.remote?.tts.remote).toBe(true);
  expect(status.remote?.tts.host).toBe('api.openai.com');
  expect(status.pending).toContain('tts.provider');
});

// A key set and no voice named is the one state where the key is there and the
// speaker still cannot start.
it('serves a remote speaker with a key and no voice named', async () => {
  const status = await at('no-voice-named');
  expect((tts(status).remote as Record<string, unknown>).voice).toBe('');
  expect(status.remote?.tts.remote).toBe(true);
  expect(status.remote?.tts.key_present).toBe(true);
});

it('serves a remote speaker with no key', async () => {
  const status = await at('no-voice');
  expect(status.remote?.tts.remote).toBe(true);
  expect(status.remote?.tts.key_present).toBe(false);
});

// A speaker that never started, and the flip away from it still waiting. The
// panel's note and the Voice lead each say one sentence for this pair alone.
it('serves a speaker that never started with its flip still waiting', async () => {
  const status = await at('to-local-not-started');
  expect(tts(status).provider).toBe('local');
  expect(status.remote?.tts.remote).toBe(true);
  expect(status.remote?.tts.speaker_started).toBe(false);
  expect(status.pending).toContain('tts.provider');
});
