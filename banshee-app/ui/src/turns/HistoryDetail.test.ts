import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { forgetCopy } from '../lib/copy';
import { dictated } from '../lib/history.mock';
import type { HistoryRow } from '../lib/tauri';
import HistoryDetail from './HistoryDetail.svelte';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { copyText } from '../lib/tauri';

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
  ...extra,
});

const show = (row: HistoryRow, goListening = vi.fn()) =>
  render(HistoryDetail, { row, now: NOW, goListening });

// The first child is the label; a copy button's second is the hidden width holder.
const buttons = () =>
  screen.queryAllByRole('button').map((b) => (b.firstElementChild ?? b).textContent?.trim());

beforeEach(() => {
  forgetCopy();
  vi.mocked(copyText).mockClear();
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
  ])('has no axe violations for %s', async (_, row) => {
    const { container } = show(row);
    const result = await axe.run(container);
    expect(result.violations).toEqual([]);
  });
});
