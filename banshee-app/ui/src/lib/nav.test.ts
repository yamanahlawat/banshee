import { expect, it } from 'vitest';
import { WAITING_SAYS, WAITS_FOR_ITS_FILE } from './copy';
import { PAGES, SETTINGS, waitingOn } from './nav';

it('lists the pages in sidebar order, with the settings last', () => {
  expect(PAGES).toEqual([
    'History',
    'Agents',
    'Listening',
    'Speaking',
    'Hotkey',
    'Feedback',
    'Saved history',
  ]);
  expect(PAGES.slice(-SETTINGS.length)).toEqual(SETTINGS);
});

it('names each page that holds a waiting key', () => {
  expect(waitingOn(['audio.hotkey_mode', 'tts.remote.model', 'stt.provider'])).toEqual({
    Listening: WAITING_SAYS,
    Speaking: WAITING_SAYS,
    Hotkey: WAITING_SAYS,
  });
});

it('says a key waits for its file in the words its row uses', () => {
  expect(waitingOn(['tts.voice'])).toEqual({ Speaking: WAITS_FOR_ITS_FILE });
  expect(waitingOn(['stt.model'])).toEqual({ Listening: WAITS_FOR_ITS_FILE });
});

// A file arrives on its own; a key Banshee could not use needs the person.
it('says could not use it yet when a page holds both kinds', () => {
  expect(waitingOn(['tts.voice', 'tts.remote.model'])).toEqual({ Speaking: WAITING_SAYS });
  expect(waitingOn(['stt.vad_threshold', 'stt.model'])).toEqual({ Listening: WAITING_SAYS });
});

it('splits the audio table between the pages that write it', () => {
  expect(waitingOn(['audio.input_device'])).toEqual({ Listening: WAITING_SAYS });
  expect(waitingOn(['audio.keep_open'])).toEqual({ Listening: WAITING_SAYS });
  expect(waitingOn(['audio.hotkey'])).toEqual({ Hotkey: WAITING_SAYS });
  expect(waitingOn(['audio.barge_in'])).toEqual({ Hotkey: WAITING_SAYS });
});

it('reaches Feedback, Saved history and the memory row', () => {
  expect(waitingOn(['feedback.mode'])).toEqual({ Feedback: WAITING_SAYS });
  expect(waitingOn(['daemon.save_history'])).toEqual({ 'Saved history': WAITING_SAYS });
  expect(waitingOn(['models.unload_after'])).toEqual({ Listening: WAITING_SAYS });
});

it('marks no page for a key no page writes', () => {
  expect(waitingOn(['sttx.model', 'daemon.socket', 'audio.hotkeys'])).toEqual({});
});
