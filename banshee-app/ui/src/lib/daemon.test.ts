import { describe, expect, it } from 'vitest';
import ready from '../mocks/ready.json';
import remote from '../mocks/remote.json';
import permissions from '../mocks/permissions.json';
import recording from '../mocks/recording.json';
import armed from '../mocks/armed.json';
import transcribing from '../mocks/transcribing.json';
import speaking from '../mocks/speaking.json';
import notRunning from '../mocks/not-running.json';
import pendingCues from '../mocks/pending-cues.json';
import {
  deviceLabel,
  downloadLine,
  empty,
  endsTheRun,
  fixGroups,
  lampForm,
  liveFrom,
  markPending,
  percent,
  spokenProgress,
  reduceLive,
  reduceStatus,
  shownFloat,
  stateWord,
  listeningFacts,
  speechFacts,
  type Blocker,
  type BlockerKind,
  type Status,
} from './daemon';

describe('the state word', () => {
  it('is Ready on a clear machine', () => {
    expect(stateWord(reduceStatus(empty(), ready))).toBe('Ready');
  });
  it('is Not ready while a permission is missing', () => {
    expect(
      stateWord(
        reduceStatus(empty(), { ...permissions, blockers: permissions.blockers as Blocker[] }),
      ),
    ).toBe('Not ready');
  });
  it('is Recording when the daemon says so, whatever status said', () => {
    const state = reduceLive(reduceStatus(empty(), ready), recording);
    expect(stateWord(state)).toBe('Recording');
    expect(lampForm('Recording')).toBe('recording');
  });
  it('is Listening while armed, because the daemon holds the microphone open then too', () => {
    const state = reduceLive(reduceStatus(empty(), ready), armed);
    expect(state.live.recording).toBe(true);
    expect(stateWord(state)).toBe('Listening');
  });
  it('is Working while transcribing, even though the mode has not gone idle', () => {
    const state = reduceLive(reduceStatus(empty(), ready), transcribing);
    expect(stateWord(state)).toBe('Working');
  });
  it('is Speaking when the daemon says so', () => {
    const state = reduceLive(reduceStatus(empty(), ready), speaking);
    expect(stateWord(state)).toBe('Speaking');
    expect(lampForm('Speaking')).toBe('speaking');
  });
  it('is Not running when the socket is down', () => {
    expect(stateWord({ ...reduceStatus(empty(), ready), down: 'closed' })).toBe('Not running');
    expect(lampForm('Not running')).toBe('notrunning');
  });
  it('is Not running when the mock reply itself says so', () => {
    expect(stateWord(reduceStatus(empty(), notRunning))).toBe('Not running');
  });
  it('stays Not running when a live event clears down but status is still stale', () => {
    // reduceLive always clears `down`; the running flag on the last status is
    // what keeps the word right until a fresh status arrives.
    const stale = reduceLive(reduceStatus(empty(), notRunning), { armed: false });
    expect(stale.down).toBeNull();
    expect(stateWord(stale)).toBe('Not running');
  });
});

describe('pending', () => {
  it('is whatever the daemon says waits for a restart', () => {
    const state = reduceStatus(empty(), pendingCues);
    expect(state.pending.has('audio.cues.enabled')).toBe(true);
  });
  it('clears when the daemon stops reporting the key', () => {
    let state = markPending(reduceStatus(empty(), pendingCues), ['stt.language']);
    expect(state.pending.has('stt.language')).toBe(true);
    state = reduceStatus(state, ready);
    expect(state.pending.has('stt.language')).toBe(false);
    expect(state.pending.has('audio.cues.enabled')).toBe(false);
  });
});

describe('the status reply carries the live flags', () => {
  it('reports Speaking from a status read alone', () => {
    expect(stateWord(reduceStatus(empty(), { ...ready, speaking: true }))).toBe('Speaking');
  });
  it('keeps what it holds for a flag the reply omits', () => {
    const held = reduceLive(empty(), { speaking: true });
    const { speaking: _omitted, ...withoutSpeaking } = ready;
    expect(reduceStatus(held, withoutSpeaking as never).live.speaking).toBe(true);
  });
  it('takes only the live flags, not the rest of the reply', () => {
    // `remote.json` carries every live flag, so the comparison is not filtered
    // down to what the mock happens to hold.
    expect(Object.keys(liveFrom(remote)).sort()).toEqual(Object.keys(empty().live).sort());
  });
});

describe('the fix groups', () => {
  const model = (id: string): Blocker => ({
    kind: 'model',
    id,
    name: id,
    consequence: 'c',
    fix: 'run: banshee setup',
  });
  const grant = (id: string): Blocker => ({
    kind: 'permission',
    id,
    name: id,
    consequence: 'c',
    fix: 'grant it',
  });
  it('puts every missing model under the one row that downloads them all', () => {
    const groups = fixGroups([model('a.bin'), model('b.onnx')]);
    expect(groups.length).toBe(1);
    expect(groups[0].length).toBe(2);
  });
  it('keeps a permission off the row that downloads the models', () => {
    const groups = fixGroups([grant('accessibility'), model('a.bin'), model('b.onnx')]);
    expect(groups.map((group) => group.length)).toEqual([1, 2]);
  });
});

describe('deviceLabel', () => {
  it('names the device the daemon opened for its own word', () => {
    expect(deviceLabel('MacBook Pro Microphone')).toBe('Default (MacBook Pro Microphone)');
  });
  it('says the word alone before the daemon opens anything', () => {
    expect(deviceLabel(null)).toBe('Default');
  });
});

describe('shownFloat', () => {
  it('drops the tail an f32 leaves on a config float', () => {
    expect(shownFloat(1.2000000476837158)).toBe(1.2);
    expect(shownFloat(0.550000011920929)).toBe(0.55);
  });
  it('leaves a value the slider can reach untouched', () => {
    expect(shownFloat(0.5)).toBe(0.5);
    expect(shownFloat(2)).toBe(2);
  });
});

it('says which file is in flight and how far it has come', () => {
  expect(
    downloadLine({
      label: 'Speech model',
      model: 'ggml-x.bin',
      index: 1,
      count: 4,
      bytes: 40,
      total: 100,
      state: 'downloading',
    }),
  ).toBe('Speech model, 1 of 4 · 40%');
});

// A daemon older than the label and count fields sends neither, and a run has
// at least one file, so a zero count has no place to report.
it('falls back to the filename when the daemon reports no place', () => {
  expect(
    downloadLine({ model: 'silero_vad.onnx', bytes: 50, total: 200, state: 'downloading' }),
  ).toBe('silero_vad.onnx · 25%');
});

// No Content-Length means no bar to draw, so it counts what has arrived.
it('counts megabytes when the server sent no length', () => {
  expect(
    downloadLine({ model: 'kokoro.onnx', bytes: 5 * 1_048_576, total: null, state: 'downloading' }),
  ).toBe('kokoro.onnx · 5 MB');
});

it('never reports past a hundred percent', () => {
  expect(percent(120, 100)).toBe(100);
  expect(percent(10, null)).toBeNull();
  expect(percent(10, 0)).toBeNull();
});

// The daemon blocks on two files and fetches four, so the blocking two land
// while the rest are still coming. Being unblocked is not being finished.
it('ends the run on its last file, not when the daemon stops being blocked', () => {
  const tick = { model: 'kokoro.onnx', bytes: 1, total: 2, index: 3, count: 4 };
  expect(endsTheRun({ ...tick, state: 'downloading' })).toBe(false);
  expect(endsTheRun({ ...tick, state: 'done' })).toBe(false);
  expect(endsTheRun({ ...tick, index: 4, state: 'done' })).toBe(true);
  expect(endsTheRun({ ...tick, index: 4, state: 'failed' })).toBe(true);
});

// A daemon older than the count field names no last file, so any terminal
// report has to end the run or the window would say Downloading for ever.
it('ends the run on any terminal report when the daemon sends no count', () => {
  expect(endsTheRun({ model: 'x.bin', bytes: 1, total: 2, state: 'done' })).toBe(true);
  expect(endsTheRun({ model: 'x.bin', bytes: 1, total: 2, state: 'downloading' })).toBe(false);
});

// download_all carries on past a bad file, so the line has to say which one
// failed or the person retries blind.
it('says when a file failed rather than showing its last percentage', () => {
  expect(
    downloadLine({
      label: 'Voice detection',
      model: 'silero_vad.onnx',
      index: 2,
      count: 4,
      bytes: 0,
      total: null,
      state: 'failed',
    }),
  ).toBe('Voice detection, 2 of 4 · failed');
});

// The daemon reports each percent and a live region reads every change it is
// given, so an 862 MB run would speak about eight hundred times. What is said
// aloud steps in quarters and holds the same words between two steps.
it('says a download aloud in quarters, and the same words in between', () => {
  const tick = {
    label: 'Speech model',
    model: 'ggml-x.bin',
    index: 1,
    count: 4,
    total: 100,
    state: 'downloading' as const,
  };
  expect(spokenProgress({ ...tick, bytes: 26 })).toBe(spokenProgress({ ...tick, bytes: 49 }));
  expect(spokenProgress({ ...tick, bytes: 26 })).not.toBe(spokenProgress({ ...tick, bytes: 51 }));
  expect(spokenProgress({ ...tick, bytes: 51 })).toBe('Speech model, 1 of 4 · 50%');
});

// A failure is the one report a person has to hear when it happens, not at the
// next quarter.
it('says a failed file aloud whatever the percentage', () => {
  expect(spokenProgress({ model: 'silero_vad.onnx', bytes: 3, total: 100, state: 'failed' })).toBe(
    'silero_vad.onnx · failed',
  );
});

// With no length to measure against there is no progress to say, so the file
// names itself once and then holds still.
it('names the file once when the server sent no length', () => {
  const tick = { model: 'kokoro.onnx', total: null, state: 'downloading' as const };
  expect(spokenProgress({ ...tick, bytes: 5 * 1_048_576 })).toBe('kokoro.onnx');
  expect(spokenProgress({ ...tick, bytes: 90 * 1_048_576 })).toBe('kokoro.onnx');
});

// Listening means an agent has stopped and is waiting for an answer. It shared
// a silhouette with Ready, which is the state where doing nothing is correct.
it('gives Listening a form of its own', () => {
  expect(lampForm('Listening')).toBe('listening');
  expect(lampForm('Ready')).toBe('idle');
});

it('carries the last error in the live state and clears it', () => {
  const failed = reduceLive(empty(), { last_error: 'the remote listener refused the key' });
  expect(failed.live.last_error).toBe('the remote listener refused the key');
  expect(reduceLive(failed, { last_error: null }).live.last_error).toBeNull();
});

it('reads the last error off a status reply', () => {
  expect(liveFrom({ running: true, last_error: 'x' } as never).last_error).toBe('x');
});

describe('the facts each side reports', () => {
  const NO_KEYS = new Set<string>();
  const KOKORO = [{ id: 'af_sky', name: 'Sky' }];
  const running = () => reduceStatus(empty(), remote as unknown as Status);

  it('reads the listener the daemon runs, and the server the config asks for', () => {
    const facts = listeningFacts(running(), NO_KEYS);
    expect(facts.remote).toBe(remote.remote.stt.remote);
    expect(facts.host).toBe(remote.remote.stt.host);
    expect(facts.keyPresent).toBe(remote.remote.stt.key_present);
    expect(facts.willUse).toBe(new URL(remote.config.stt.remote.base_url).hostname);
    expect(facts.device).toBe(remote.audio_device);
    expect(facts.live).toBe(true);
  });

  it('reads the speaker the daemon runs, and not the provider the config names', () => {
    const facts = speechFacts(running(), NO_KEYS, KOKORO);
    expect(remote.config.tts.provider).toBe('local');
    expect(facts.remote).toBe(remote.remote.tts.remote);
    expect(facts.started).toBe(remote.remote.tts.speaker_started);
    expect(facts.keyPresent).toBe(remote.remote.tts.key_present);
    expect(facts.host).toBe(remote.remote.tts.host);
  });

  // The daemon answers an empty host when the address the side runs on is no
  // URL, and every sentence over these facts tests for an absent name.
  it('names no host when the daemon answers an empty one', () => {
    const state = reduceStatus(empty(), {
      ...(remote as unknown as Status),
      remote: {
        stt: { remote: true, host: '', key_present: true },
        tts: { remote: true, host: '', speaker_started: true, key_present: true },
      },
    });
    expect(listeningFacts(state, NO_KEYS).host).toBeNull();
    expect(speechFacts(state, NO_KEYS, KOKORO).host).toBeNull();
  });

  it('names no server when the config holds no URL to read one from', () => {
    const state = reduceStatus(empty(), {
      ...(remote as unknown as Status),
      config: { stt: { remote: { base_url: 'api.openai.com' } }, tts: {} },
    });
    expect(listeningFacts(state, NO_KEYS).willUse).toBeNull();
    expect(speechFacts(state, NO_KEYS, KOKORO).willUse).toBeNull();
  });

  // Kokoro's list holds no entry for a voice a server names, so the name comes
  // from the table the speaker in force reads.
  it('takes a local voice from the list and a remote one from its own table', () => {
    expect(speechFacts(running(), NO_KEYS, KOKORO).voiceName).toBe('Sky');

    const spoken = reduceStatus(empty(), {
      ...(remote as unknown as Status),
      remote: {
        stt: remote.remote.stt,
        tts: { remote: true, host: 'api.openai.com', speaker_started: true, key_present: true },
      },
      config: {
        ...remote.config,
        tts: { ...remote.config.tts, remote: { ...remote.config.tts.remote, voice: 'marin' } },
      },
    });
    expect(speechFacts(spoken, NO_KEYS, KOKORO).voiceName).toBe('marin');
  });

  // A voice the daemon holds that Kokoro's list does not name still reaches the
  // reader, because the list arrives from a call of its own and can be empty.
  it('falls back to the voice the config names', () => {
    expect(speechFacts(running(), NO_KEYS, []).voiceName).toBe(remote.config.tts.voice);
  });

  it('gives each side only the restart its own provider waits on', () => {
    const state = running();
    expect(listeningFacts(state, new Set(['stt.provider'])).pending).toBe(true);
    expect(speechFacts(state, new Set(['stt.provider']), KOKORO).pending).toBe(false);
    expect(listeningFacts(state, new Set(['tts.provider'])).pending).toBe(false);
    expect(speechFacts(state, new Set(['tts.provider']), KOKORO).pending).toBe(true);
  });

  it('says neither side is live while the daemon is down', () => {
    const state = reduceStatus(empty(), notRunning as Status);
    expect(listeningFacts(state, NO_KEYS).live).toBe(false);
    expect(speechFacts(state, NO_KEYS, KOKORO).live).toBe(false);
  });

  // A pipeline blocker with another id, such as the Linux typer, stops the
  // typing and not the listener.
  it('names the kind of the recording blocker, and reads no other blocker as one', () => {
    const stoppedBy = (kind: BlockerKind, id = 'recording_pipeline') =>
      listeningFacts(
        reduceStatus(empty(), {
          ...(remote as unknown as Status),
          blockers: [
            {
              kind,
              id,
              name: 'The listener never started',
              consequence: 'nothing is heard',
              fix: 'restart it',
            },
          ],
        }),
        NO_KEYS,
      ).stoppedBy;
    expect(stoppedBy('pipeline')).toBe('pipeline');
    expect(stoppedBy('provider')).toBe('provider');
    expect(stoppedBy('keyfile')).toBe('keyfile');
    expect(stoppedBy('pipeline', 'wayland_typer')).toBe(null);

    const ungranted = reduceStatus(empty(), {
      ...permissions,
      blockers: permissions.blockers as Blocker[],
    });
    expect(ungranted.status?.blockers).toHaveLength(1);
    expect(listeningFacts(ungranted, NO_KEYS).stoppedBy).toBeNull();
  });
});
