import { render, screen } from '@testing-library/svelte';
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
