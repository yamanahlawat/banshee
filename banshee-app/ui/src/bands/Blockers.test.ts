import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Blockers from './Blockers.svelte';
import permissions from '../fixtures/permissions.json';
import type { Blocker } from '../lib/daemon';

const at = (bytes: number) => ({
  label: 'Speech model',
  model: 'ggml-x.bin',
  index: 1,
  count: 4,
  bytes,
  total: 100,
  state: 'downloading' as const,
});

// The daemon reports each percent. A reader watching the screen wants all of
// them; a reader listening wants four.
it('shows every percent and says only the quarters', async () => {
  const { container, rerender } = render(Blockers, {
    blockers: [],
    download: at(26),
    restart: () => {},
  });
  const shown = () => container.querySelector('.progress')?.textContent?.trim();
  const said = () => container.querySelector('[aria-live="polite"]')?.textContent?.trim();

  expect(shown()).toContain('26%');
  const first = said();

  await rerender({ blockers: [], download: at(49), restart: () => {} });
  expect(shown()).toContain('49%');
  expect(said()).toBe(first);

  await rerender({ blockers: [], download: at(51), restart: () => {} });
  expect(said()).not.toBe(first);
});

// Two permission blockers draw the same button. Heard one after the other with
// no pane in either name, they are the same button twice.
it('names the pane each System Settings button opens', () => {
  const { getByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
  });
  expect(getByRole('button', { name: /Open System Settings for Accessibility/ })).toBeTruthy();
  expect(getByRole('button', { name: /Open System Settings for Input Monitoring/ })).toBeTruthy();
});

// The title of a blocker is the most important line on the surface when one is
// there, and a span cannot be reached by heading.
it('titles a blocker with a heading', () => {
  const { getAllByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
  });
  expect(getAllByRole('heading').map((h) => h.textContent?.trim())).toEqual([
    'Accessibility',
    'Input Monitoring',
  ]);
});
