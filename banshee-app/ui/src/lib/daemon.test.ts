import { describe, expect, it } from 'vitest';
import ready from '../fixtures/ready.json';
import permissions from '../fixtures/permissions.json';
import recording from '../fixtures/recording.json';
import armed from '../fixtures/armed.json';
import transcribing from '../fixtures/transcribing.json';
import speaking from '../fixtures/speaking.json';
import notRunning from '../fixtures/not-running.json';
import pendingCues from '../fixtures/pending-cues.json';
import { STATIONS, checklist, deviceLabel, empty, fixGroups, fixProse, lampForm, liveFrom, markPending, needleAt, reduceLive, reduceStatus, shownFloat, stateWord } from './daemon';

describe('the state word', () => {
  it('is Ready on a clear machine', () => {
    expect(stateWord(reduceStatus(empty(), ready))).toBe('Ready');
  });
  it('is Not ready while a permission is missing', () => {
    expect(stateWord(reduceStatus(empty(), permissions))).toBe('Not ready');
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

describe('the checklist', () => {
  it('marks every station clear on a clear machine and Try it todo', () => {
    const rows = checklist(reduceStatus(empty(), ready));
    expect(rows.map((r) => r.station)).toEqual(['Running', 'Microphone', 'Permissions', 'Models', 'Try it']);
    expect(rows.map((r) => r.state)).toEqual(['clear', 'clear', 'clear', 'clear', 'todo']);
  });
  it('puts a missing grant under Permissions with the daemon\'s own words', () => {
    const rows = checklist(reduceStatus(empty(), permissions));
    const perms = rows.find((r) => r.station === 'Permissions')!;
    expect(perms.state).toBe('blocked');
    expect(perms.blockers[0].fix.length).toBeGreaterThan(0);
  });
  it('puts a blocker of a kind the daemon does not emit under Running, so it never vanishes', () => {
    const status = { ...ready, blockers: [{ kind: 'mystery', id: 'x', name: 'X', consequence: 'y', fix: 'z' }] };
    const rows = checklist(reduceStatus(empty(), status));
    const running = rows.find((r) => r.station === 'Running')!;
    expect(running.state).toBe('blocked');
    expect(running.blockers).toHaveLength(1);
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

describe('the needle', () => {
  it('rests on the first station that is not clear', () => {
    expect(needleAt(checklist(reduceStatus(empty(), permissions)))).toBe(
      STATIONS.indexOf('Permissions'),
    );
  });
  it('reaches Try it on a machine with nothing to fix', () => {
    expect(needleAt(checklist(reduceStatus(empty(), ready)))).toBe(STATIONS.indexOf('Try it'));
  });
  it('rests on Running when the daemon is down', () => {
    const down = { ...reduceStatus(empty(), permissions), down: 'not running' };
    expect(needleAt(checklist(down))).toBe(STATIONS.indexOf('Running'));
  });
});

describe('the fix groups', () => {
  const model = (id: string) => ({ kind: 'model', id, name: id, consequence: 'c', fix: 'run: banshee setup' });
  const grant = (id: string) => ({ kind: 'permission', id, name: id, consequence: 'c', fix: 'grant it' });
  it('puts every missing model under the one row that downloads them all', () => {
    const groups = fixGroups([model('a.bin'), model('b.onnx')]);
    expect(groups.length).toBe(1);
    expect(groups[0].length).toBe(2);
  });
  it('keeps a permission on its own row, because each names its own pane', () => {
    expect(fixGroups([grant('accessibility'), grant('input_monitoring')]).length).toBe(2);
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
