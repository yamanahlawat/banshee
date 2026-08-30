import { fireEvent, render, screen } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Segmented from './Segmented.svelte';

const OPTIONS = [
  { value: 'fast', label: 'Fast' },
  { value: 'balanced', label: 'Balanced' },
  { value: 'quality', label: 'Quality' },
];

// NOT COVERED HERE: that the chosen segment is drawn differently. jsdom resolves
// no scoped stylesheet, so an assertion on its background passes with the rule
// deleted. The rule now lives in Segmented.svelte beside the attribute it
// selects, which is what keeps the two from drifting.
it('checks the chosen option and no other', () => {
  render(Segmented, {
    label: 'Transcription',
    value: 'balanced',
    options: OPTIONS,
    change: () => {},
  });
  expect(screen.getByRole('radiogroup', { name: 'Transcription' })).toBeTruthy();
  expect(screen.getByRole('radio', { name: 'Balanced' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByRole('radio', { name: 'Fast' }).getAttribute('aria-checked')).toBe('false');
  expect(screen.getByRole('radio', { name: 'Quality' }).getAttribute('aria-checked')).toBe('false');
});

// A radiogroup is one tab stop and the arrows move inside it. Without that the
// group tells a screen reader it is one of three and then refuses to move.
it('moves the choice and the focus with the arrow keys, and wraps at the end', async () => {
  let chosen = '';
  render(Segmented, {
    label: 'Transcription',
    value: 'quality',
    options: OPTIONS,
    change: (next: string) => (chosen = next),
  });
  await fireEvent.keyDown(screen.getByRole('radio', { name: 'Quality' }), { key: 'ArrowRight' });
  expect(chosen).toBe('fast');
  // The chosen option is the only tab stop, so the focus has to follow it out
  // or the next Tab leaves from a cell nobody is on.
  expect(document.activeElement).toBe(screen.getByRole('radio', { name: 'Fast' }));

  await fireEvent.keyDown(screen.getByRole('radio', { name: 'Quality' }), { key: 'ArrowLeft' });
  expect(chosen).toBe('balanced');
});

// Three tab stops make the group a corridor. One makes it a control.
it('puts the only tab stop on the chosen option', () => {
  render(Segmented, {
    label: 'Transcription',
    value: 'balanced',
    options: OPTIONS,
    change: () => {},
  });
  expect(screen.getAllByRole('radio').map((r) => (r as HTMLElement).tabIndex)).toEqual([-1, 0, -1]);
});
