import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { get } from 'svelte/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { announcement, forgetCopy, problem } from '../lib/copy';
import { daemon, empty, reduceStatus, type Status } from '../lib/daemon';
import { retrying, table } from '../lib/history';
import { stubMedia } from '../lib/media.mock';
import { dictated } from '../lib/history.mock';
import type { HistoryRow } from '../lib/tauri';
import HistoryDetail from './HistoryDetail.svelte';
import ready from '../mocks/ready.json';
import remote from '../mocks/remote.json';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { copyText, historyAudio, retry } from '../lib/tauri';

const NOW = new Date(2026, 10, 6, 15, 0);
const STAMP = new Date(2026, 10, 6, 14, 32).toISOString();

const asked = (outcome: HistoryRow['outcome'], extra: Partial<HistoryRow> = {}): HistoryRow => ({
  id: 7,
  kind: 'question',
  question: 'Which branch should I use?',
  text: null,
  outcome,
  reason: null,
  timestamp: STAMP,
  audio: null,
  ...extra,
});

const show = (row: HistoryRow, goListening = vi.fn(), query = '') =>
  render(HistoryDetail, { row, now: NOW, goListening, query });

// The first child is the label; a copy button's second is the hidden width holder.
const buttons = () =>
  screen.queryAllByRole('button').map((b) => (b.firstElementChild ?? b).textContent?.trim());

beforeEach(() => {
  forgetCopy();
  daemon.set(empty());
  retrying.set(new Set());
  vi.mocked(copyText).mockClear();
  vi.mocked(retry).mockReset();
});

describe('the frame', () => {
  it('is a region named Selected row, led by a date heading that takes focus', () => {
    const { component } = show(dictated(1, 'Open the pull request.', STAMP));
    const region = screen.getByRole('region', { name: 'Selected row' });
    const heading = screen.getByRole('heading', { name: 'Today at 14:32' });
    expect(region.contains(heading)).toBe(true);
    expect(heading.getAttribute('tabindex')).toBe('-1');
    component.focus();
    expect(document.activeElement).toBe(heading);
    expect(region.getAttribute('aria-live')).toBeNull();
  });

  it('names an earlier day', () => {
    show(dictated(1, 'x', new Date(2026, 10, 5, 9, 5).toISOString()));
    expect(screen.getByRole('heading', { name: 'Yesterday at 09:05' })).toBeTruthy();
  });
});

describe('a dictation', () => {
  it('shows its text with one Copy', async () => {
    show(dictated(1, 'Open the pull request.', STAMP));
    expect(screen.getByText('Open the pull request.')).toBeTruthy();
    expect(buttons()).toEqual(['Copy']);
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    expect(copyText).toHaveBeenCalledWith('Open the pull request.');
  });

  it('reads Copied after a copy', async () => {
    show(dictated(1, 'Yes.', STAMP));
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Copied' })).toBeTruthy());
  });
});

describe('a failed dictation', () => {
  const failed = (reason: string | null) => ({
    ...dictated(2, '', STAMP),
    text: null,
    outcome: 'failed' as const,
    reason,
  });

  it('says it failed, gives the reason and opens Listening', async () => {
    const goListening = vi.fn();
    show(failed('The server refused the key.'), goListening);
    expect(screen.getByText('Transcription failed')).toBeTruthy();
    expect(screen.getByText('The server refused the key.')).toBeTruthy();
    expect(buttons()).toEqual(['Open Listening']);
    await fireEvent.click(screen.getByRole('button', { name: 'Open Listening' }));
    expect(goListening).toHaveBeenCalledOnce();
  });

  it('draws no reason line when the reason is null', () => {
    const { container } = show(failed(null));
    expect(screen.getByText('Transcription failed')).toBeTruthy();
    expect(container.querySelector('.note')).toBeNull();
    expect(buttons()).toEqual(['Open Listening']);
  });
});

describe('a question', () => {
  it('answered: question, rule, answer, and both copies', async () => {
    const { container } = show(asked('transcribed', { text: 'The release branch.' }));
    expect(screen.getByText('Which branch should I use?')).toBeTruthy();
    expect(screen.getByText('The release branch.')).toBeTruthy();
    expect(container.querySelector('hr')).toBeTruthy();
    expect(buttons()).toEqual(['Copy answer', 'Copy question']);
    await fireEvent.click(screen.getByRole('button', { name: 'Copy question' }));
    expect(copyText).toHaveBeenCalledWith('Which branch should I use?');
    await fireEvent.click(screen.getByRole('button', { name: 'Copy answer' }));
    expect(copyText).toHaveBeenCalledWith('The release branch.');
  });

  it.each([
    ['no_answer', 'No answer heard'],
    ['closed', 'Closed before you answered'],
  ] as const)('%s: question, rule, flag, Copy question', (outcome, flag) => {
    const { container } = show(asked(outcome));
    expect(screen.getByText('Which branch should I use?')).toBeTruthy();
    expect(container.querySelector('hr')).toBeTruthy();
    expect(screen.getByText(flag)).toBeTruthy();
    expect(buttons()).toEqual(['Copy question']);
  });

  it('failed: flag and reason', () => {
    show(asked('failed', { reason: 'The audio was empty.' }));
    expect(screen.getByText('Not transcribed')).toBeTruthy();
    expect(screen.getByText('The audio was empty.')).toBeTruthy();
    expect(buttons()).toEqual(['Copy question']);
  });

  it('failed with a null reason draws no empty line', () => {
    const { container } = show(asked('failed'));
    expect(screen.getByText('Not transcribed')).toBeTruthy();
    expect(container.querySelector('.note')).toBeNull();
  });

  it('unasked: flag and what the agent was told', () => {
    show(asked('unasked', { reason: 'no microphone' }));
    expect(screen.getByText('Not asked aloud')).toBeTruthy();
    expect(screen.getByText('The agent was told: no microphone')).toBeTruthy();
    expect(buttons()).toEqual(['Copy question']);
  });

  it('unasked with a null reason draws no empty line', () => {
    const { container } = show(asked('unasked'));
    expect(screen.getByText('Not asked aloud')).toBeTruthy();
    expect(container.querySelector('.note')).toBeNull();
  });
});

describe('kept audio', () => {
  const kept = (row: HistoryRow, seconds: number, until: string | null): HistoryRow => ({
    ...row,
    audio: { seconds, until },
  });
  const LATER = new Date(2026, 10, 13, 14, 32).toISOString();
  const EARLIER = new Date(2026, 10, 6, 9, 0).toISOString();
  const failed = { ...dictated(2, '', STAMP), text: null, outcome: 'failed' as const };
  const play = () => screen.queryByRole('button', { name: /^Play audio/ });

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('a row with audio shows the player and the date', () => {
    show(kept(dictated(1, 'Open the pull request.', STAMP), 6, LATER));
    expect(screen.getByRole('button', { name: 'Play audio, 6 seconds' })).toBeTruthy();
    expect(screen.getByText('Audio kept until 13 November')).toBeTruthy();
    expect(buttons()).toEqual(['Play audio, 6 seconds', 'Copy', 'Retry']);
  });

  it('forever says until you clear it', () => {
    show(kept(dictated(1, 'Open the pull request.', STAMP), 6, null));
    expect(play()).toBeTruthy();
    expect(screen.getByText('Audio kept until you clear it')).toBeTruthy();
  });

  it('a row past its date shows no player', () => {
    show(kept(dictated(1, 'Open the pull request.', STAMP), 6, EARLIER));
    expect(play()).toBeNull();
    expect(screen.queryByText(/Audio kept/)).toBeNull();
  });

  it('a row whose date passed earlier today shows no player', () => {
    const until = new Date(2026, 10, 6, 14, 0).toISOString();
    render(HistoryDetail, {
      row: kept(dictated(1, 'Open the pull request.', STAMP), 6, until),
      now: new Date(2026, 10, 6),
      goListening: vi.fn(),
    });
    expect(play()).toBeNull();
  });

  it.each([
    ['a failed dictation', failed],
    ['an answered question', asked('transcribed', { text: 'The release branch.' })],
    ['a failed question', asked('failed', { reason: 'The audio was empty.' })],
  ])('%s with audio shows the player', (_, row) => {
    show(kept(row, 4, LATER));
    expect(screen.getByRole('button', { name: 'Play audio, 4 seconds' })).toBeTruthy();
    expect(screen.queryByText('No audio was kept, so there is nothing to retry.')).toBeNull();
  });

  it('a failed dictation without audio says nothing to retry', () => {
    show(failed);
    expect(screen.getByText('No audio was kept, so there is nothing to retry.')).toBeTruthy();
    expect(buttons()).toEqual(['Open Listening']);
  });

  it('a failed dictation past its date says nothing to retry', () => {
    show(kept(failed, 4, EARLIER));
    expect(play()).toBeNull();
    expect(screen.getByText('No audio was kept, so there is nothing to retry.')).toBeTruthy();
  });

  it('a row from before shows no player', () => {
    show(dictated(1, 'Open the pull request.', STAMP));
    expect(play()).toBeNull();
    expect(screen.queryByText(/Audio kept/)).toBeNull();
    expect(screen.queryByText(/nothing to retry/)).toBeNull();
  });

  it("togglePlay plays the row's audio", async () => {
    const media = stubMedia();
    vi.mocked(historyAudio).mockResolvedValue(new ArrayBuffer(8));
    const { component } = show(kept(dictated(1, 'Yes.', STAMP), 2, null));
    await component.togglePlay();
    expect(await screen.findByRole('button', { name: 'Pause audio' })).toBeTruthy();
    expect(historyAudio).toHaveBeenCalledWith(1);
    media.restore();
  });

  describe('a row change', () => {
    const first = kept(dictated(4, 'Yes.', STAMP), 2, null);
    const second = kept(dictated(5, 'No.', STAMP), 3, null);
    let media: ReturnType<typeof stubMedia>;

    beforeEach(() => {
      vi.mocked(historyAudio).mockReset();
      vi.mocked(historyAudio).mockResolvedValue(new ArrayBuffer(8));
      media = stubMedia();
    });

    afterEach(() => {
      media.restore();
    });

    it('stops, revokes and fetches its own clip', async () => {
      const { rerender } = show(first);
      await fireEvent.click(screen.getByRole('button', { name: 'Play audio, 2 seconds' }));
      await screen.findByRole('button', { name: 'Pause audio' });

      await rerender({ row: second });
      expect(media.pause).toHaveBeenCalled();
      expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:clip-1');
      await fireEvent.click(screen.getByRole('button', { name: 'Play audio, 3 seconds' }));
      await screen.findByRole('button', { name: 'Pause audio' });
      expect(historyAudio).toHaveBeenLastCalledWith(5);
    });

    // A load still in flight for the old clip can fail after the row changed.
    it('lets go of the old clip, and its late error reports nothing', async () => {
      const { container, rerender } = show(first);
      await fireEvent.click(screen.getByRole('button', { name: 'Play audio, 2 seconds' }));
      await screen.findByRole('button', { name: 'Pause audio' });
      const old = container.querySelector('audio')!;

      await rerender({ row: second });
      expect(old.hasAttribute('src')).toBe(false);
      expect(media.load).toHaveBeenCalled();
      await fireEvent(old, new Event('error'));
      expect(get(problem)).toBe('');
    });

    it('drops a slow fetch, and the next press fetches the new clip', async () => {
      let answer: (bytes: ArrayBuffer) => void = () => {};
      vi.mocked(historyAudio).mockReturnValueOnce(new Promise((resolve) => (answer = resolve)));
      const { rerender } = show(first);
      await fireEvent.click(screen.getByRole('button', { name: 'Play audio, 2 seconds' }));
      await rerender({ row: second });
      await fireEvent.click(screen.getByRole('button', { name: 'Play audio, 3 seconds' }));
      await screen.findByRole('button', { name: 'Pause audio' });
      expect(historyAudio).toHaveBeenLastCalledWith(5);
      answer(new ArrayBuffer(8));
    });
  });
});

describe('retry', () => {
  const LATER = new Date(2026, 10, 13, 14, 32).toISOString();
  const withAudio = (row: HistoryRow): HistoryRow => ({
    ...row,
    audio: { seconds: 4, until: LATER },
  });
  const spoken = withAudio(dictated(1, 'Open the pull request.', STAMP));
  const failed = withAudio({
    ...dictated(2, '', STAMP),
    text: null,
    outcome: 'failed',
    reason: 'The server refused the key.',
  });
  const retryButton = () => screen.getByRole('button', { name: /^Retry/ });
  const held = (button: HTMLElement) =>
    button.querySelector('[aria-hidden="true"]')?.textContent?.trim();
  const settle = () => new Promise((resolve) => setTimeout(resolve));

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(NOW);
    daemon.set(reduceStatus(empty(), ready as Status));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('retry_names_the_model', () => {
    show(spoken);
    expect(buttons()).toEqual(['Play audio, 4 seconds', 'Copy', 'Retry with Parakeet Unified']);
    expect(screen.queryByText(/^Sends this recording/)).toBeNull();
  });

  it('retry_names_the_server_and_says_where_audio_goes', () => {
    daemon.set(reduceStatus(empty(), remote as Status));
    show(spoken);
    expect(retryButton().firstElementChild?.textContent).toBe(
      'Retry with the transcription server',
    );
    expect(screen.getByText('Sends this recording to api.openai.com.')).toBeTruthy();
  });

  it('the retry button is described by where the audio goes', () => {
    daemon.set(reduceStatus(empty(), remote as Status));
    show(spoken);
    expect(
      screen.getByRole('button', { description: 'Sends this recording to api.openai.com.' }),
    ).toBe(retryButton());
  });

  it('names the server while a broken microphone stops dictation', () => {
    const blocker = {
      kind: 'pipeline' as const,
      id: 'recording_pipeline',
      name: 'The microphone is not working',
      remedy: 'restart' as const,
      consequence: 'dictation cannot record',
      fix: 'plug the microphone back in',
    };
    daemon.set(reduceStatus(empty(), { ...(remote as Status), blockers: [blocker] }));
    show(spoken);
    expect(screen.getByText('Sends this recording to api.openai.com.')).toBeTruthy();
  });

  it('a local listener draws no server line and no description', () => {
    show(spoken);
    expect(retryButton().getAttribute('aria-describedby')).toBeNull();
  });

  it('a failed dictation keeps Open Listening beside Retry', () => {
    show(failed);
    expect(buttons()).toEqual([
      'Play audio, 4 seconds',
      'Retry with Parakeet Unified',
      'Open Listening',
    ]);
  });

  it('a_question_row_has_no_retry', () => {
    show(withAudio(asked('transcribed', { text: 'The release branch.' })));
    expect(screen.queryByRole('button', { name: /^Retry/ })).toBeNull();
    show(withAudio(asked('failed', { reason: 'The audio was empty.' })));
    expect(screen.queryByRole('button', { name: /^Retry/ })).toBeNull();
  });

  it('a row without playable audio has no retry', () => {
    show(dictated(1, 'Open the pull request.', STAMP));
    expect(screen.queryByRole('button', { name: /^Retry/ })).toBeNull();
  });

  it('a_successful_retry_replaces_the_words_and_announces', async () => {
    const after = { ...spoken, text: 'Open the pull request now.' };
    table.set({ rows: [spoken], total: 9, loaded: true, saving: true });
    vi.mocked(retry).mockResolvedValue(after);
    const { rerender } = show(spoken);
    const button = retryButton();
    button.focus();
    await fireEvent.click(button);
    expect(retry).toHaveBeenCalledWith(1);
    await waitFor(() => expect(get(announcement)).toBe('Text replaced.'));
    expect(get(table).rows).toEqual([after]);
    expect(get(table).total).toBe(9);
    await rerender({ row: get(table).rows[0] });
    expect(screen.getByText('Open the pull request now.')).toBeTruthy();
    expect(document.activeElement).toBe(button);
    expect(button.firstElementChild?.textContent).toBe('Retry with Parakeet Unified');
  });

  it('a successful retry on a failed dictation transcribes it and keeps focus on Retry', async () => {
    const after: HistoryRow = { ...failed, text: 'Ship it.', outcome: 'transcribed', reason: null };
    table.set({ rows: [failed], total: 1, loaded: true, saving: true });
    vi.mocked(retry).mockResolvedValue(after);
    const { rerender } = show(failed);
    const button = retryButton();
    button.focus();
    await fireEvent.click(button);
    await waitFor(() => expect(get(announcement)).toBe('Transcribed.'));
    await rerender({ row: get(table).rows[0] });
    expect(screen.queryByText('Transcription failed')).toBeNull();
    expect(screen.getByText('Ship it.')).toBeTruthy();
    expect(buttons()).toEqual(['Play audio, 4 seconds', 'Copy', 'Retry with Parakeet Unified']);
    expect(document.activeElement).toBe(button);
  });

  it('a_failed_retry_reports_and_keeps_the_words', async () => {
    table.set({ rows: [spoken], total: 1, loaded: true, saving: true });
    vi.mocked(retry).mockRejectedValue({ message: 'The model returned no text.' });
    show(spoken);
    await fireEvent.click(retryButton());
    await waitFor(() =>
      expect(get(problem)).toBe(
        'The retry failed. The model returned no text. The text is unchanged.',
      ),
    );
    expect(get(table).rows).toEqual([spoken]);
    expect(screen.getByText('Open the pull request.')).toBeTruthy();
    expect(retryButton().firstElementChild?.textContent).toBe('Retry with Parakeet Unified');
  });

  it('a failed retry reads a transport message as a sentence', async () => {
    table.set({ rows: [spoken], total: 1, loaded: true, saving: true });
    vi.mocked(retry).mockRejectedValue({ message: 'the daemon closed the socket' });
    show(spoken);
    await fireEvent.click(retryButton());
    await waitFor(() =>
      expect(get(problem)).toBe(
        'The retry failed. The daemon closed the socket. The text is unchanged.',
      ),
    );
  });

  it('a failed retry on a failed dictation reports the reason alone', async () => {
    table.set({ rows: [failed], total: 1, loaded: true, saving: true });
    vi.mocked(retry).mockRejectedValue({ message: 'The model returned no text.' });
    show(failed);
    await fireEvent.click(retryButton());
    await waitFor(() => expect(get(problem)).toBe('The retry failed. The model returned no text.'));
    expect(get(table).rows).toEqual([failed]);
    expect(screen.getByText('Transcription failed')).toBeTruthy();
  });

  it('a detail mounted again mid-retry still reads Retrying and ignores a press', async () => {
    let answer: (row: HistoryRow) => void = () => {};
    vi.mocked(retry).mockReturnValue(new Promise((resolve) => (answer = resolve)));
    const first = show(spoken);
    await fireEvent.click(retryButton());
    first.unmount();
    show(spoken);
    const button = retryButton();
    expect(button.firstElementChild?.textContent).toBe('Retrying');
    await fireEvent.click(button);
    expect(retry).toHaveBeenCalledOnce();
    answer(spoken);
    await waitFor(() =>
      expect(button.firstElementChild?.textContent).toBe('Retry with Parakeet Unified'),
    );
  });

  it('a_second_press_while_retrying_does_nothing', async () => {
    let answer: (row: HistoryRow) => void = () => {};
    vi.mocked(retry).mockReturnValue(new Promise((resolve) => (answer = resolve)));
    show(spoken);
    const button = retryButton();
    await fireEvent.click(button);
    expect(button.firstElementChild?.textContent).toBe('Retrying');
    expect(held(button)).toBe('Retry with Parakeet Unified');
    expect(button.hasAttribute('disabled')).toBe(false);
    await fireEvent.click(button);
    expect(retry).toHaveBeenCalledOnce();
    answer(spoken);
    await settle();
    await waitFor(() =>
      expect(button.firstElementChild?.textContent).toBe('Retry with Parakeet Unified'),
    );
    expect(held(button)).toBe('Retry with Parakeet Unified');
  });

  it('the words read as pending while the retry runs, and come back with the answer', async () => {
    let answer: (row: HistoryRow) => void = () => {};
    vi.mocked(retry).mockReturnValue(new Promise((resolve) => (answer = resolve)));
    const { container } = show(spoken);
    const words = () => container.querySelector('.words') as HTMLElement;
    expect(words().classList.contains('pending')).toBe(false);
    await fireEvent.click(retryButton());
    expect(words().classList.contains('pending')).toBe(true);
    answer({ ...spoken, text: 'The new words.' });
    await settle();
    await waitFor(() => expect(words().classList.contains('pending')).toBe(false));
  });
});

describe('under a search', () => {
  it('marks the matches in the question and the answer', () => {
    const { container } = show(
      asked('transcribed', { text: 'The release branch.' }),
      vi.fn(),
      'branch',
    );
    expect([...container.querySelectorAll('mark')].map((mark) => mark.textContent)).toEqual([
      'branch',
      'branch',
    ]);
    expect(container.querySelector('.agent')?.textContent).toBe('Which branch should I use?');
  });

  it('marks nothing with no search', () => {
    const { container } = show(asked('transcribed', { text: 'The release branch.' }));
    expect(container.querySelector('mark')).toBeNull();
  });
});

describe('a Copy button', () => {
  const held = (name: string) =>
    screen.getByRole('button', { name }).querySelector('[aria-hidden="true"]')?.textContent;

  it.each([
    ['Copy', dictated(1, 'Yes.', STAMP), 'Copied'],
    ['Copy answer', asked('transcribed', { text: 'The release branch.' }), 'Copy answer'],
    ['Copy question', asked('no_answer'), 'Copy question'],
  ] as const)('%s holds the wider of its label and Copied', async (label, row, widest) => {
    show(row);
    expect(held(label)).toBe(widest);
    await fireEvent.click(screen.getByRole('button', { name: label }));
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeTruthy();
    expect(held('Copied')).toBe(widest);
  });
});

describe('accessibility', () => {
  it.each([
    ['answered', asked('transcribed', { text: 'The release branch.' })],
    ['unasked', asked('unasked', { reason: 'no microphone' })],
    ['failed dictation', { ...dictated(2, '', STAMP), text: null, outcome: 'failed' as const }],
    [
      'a dictation with audio',
      { ...dictated(1, 'Yes.', STAMP), audio: { seconds: 3, until: null } },
    ],
  ])('has no axe violations for %s', async (_, row) => {
    const { container } = show(row);
    const result = await axe.run(container);
    expect(result.violations).toEqual([]);
  });
});
