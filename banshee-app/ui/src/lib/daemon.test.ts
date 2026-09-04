import { describe, expect, it } from 'vitest';
import ready from '../fixtures/ready.json';
import permissions from '../fixtures/permissions.json';
import recording from '../fixtures/recording.json';
import armed from '../fixtures/armed.json';
import transcribing from '../fixtures/transcribing.json';
import speaking from '../fixtures/speaking.json';
import notRunning from '../fixtures/not-running.json';
import pendingCues from '../fixtures/pending-cues.json';
import {
  deviceLabel,
  downloadLine,
  empty,
  endsTheRun,
  fixGroups,
  lampForm,
  liveFrom,
  markPending,
  microphoneInUse,
  percent,
  spokenProgress,
  reduceLive,
  reduceStatus,
  shownFloat,
  stateWord,
  type Blocker,
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
  it('is Not running when the daemon fixture itself says so', () => {
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
    expect(Object.keys(liveFrom(ready)).sort()).toEqual(Object.keys(empty().live).sort());
  });
});

describe('the fix groups', () => {
  const model = (id: string) => ({
    kind: 'model',
    id,
    name: id,
    consequence: 'c',
    fix: 'run: banshee setup',
  });
  const grant = (id: string) => ({
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

// The cases `microphone_label` covers, so the two cannot drift apart.
describe('microphoneInUse', () => {
  it('names the device the daemon opened', () => {
    expect(microphoneInUse('MacBook Pro Microphone')).toBe('MacBook Pro Microphone');
  });

  it('says the stream is closed, not that the machine has no microphone', () => {
    expect(microphoneInUse(null)).toBe('Not open');
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
