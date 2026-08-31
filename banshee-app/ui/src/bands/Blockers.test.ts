import { fireEvent, render } from '@testing-library/svelte';
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
  // Two boxes with no count leave the reader counting.
  expect(getAllByRole('heading').map((h) => h.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
    '1 of 2 · Accessibility',
    '2 of 2 · Input Monitoring',
  ]);
});

// A TCC grant reaches only a process started after it lands, so opening the
// pane cannot finish the job. The box has to offer the restart that does.
it('offers the restart that makes a granted permission real', async () => {
  let restarted = 0;
  const { getByRole, queryByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => (restarted += 1),
  });
  expect(queryByRole('button', { name: /I granted it/ })).toBeNull();

  await fireEvent.click(getByRole('button', { name: /Open System Settings for Accessibility/ }));
  await fireEvent.click(getByRole('button', { name: /I granted it/ }));
  expect(restarted).toBe(1);
});

// Two boxes with no count leave the reader holding "which one did I do" across
// an app switch.
it('numbers each thing it still needs', () => {
  const { getAllByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
  });
  const heads = getAllByRole('heading').map((h) => h.textContent?.replace(/\s+/g, ' ').trim());
  expect(heads[0]).toMatch(/^1 of 2/);
  expect(heads[1]).toMatch(/^2 of 2/);
});

// First run is the one moment a person asks what Banshee is, and it is
// derived rather than stored: nothing dictated yet.
it('says what Banshee is on a first run, and not afterwards', () => {
  const { queryByText, rerender } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
    first: true,
  });
  expect(queryByText(/types what you say/)).toBeTruthy();

  rerender({ blockers: permissions.blockers as Blocker[], restart: () => {}, first: false });
  expect(queryByText(/types what you say/)).toBeNull();
});

// The daemon streams the percent, so the bar draws a real value rather than
// standing in for one.
it('draws the download against its own length', () => {
  const { container } = render(Blockers, {
    blockers: [],
    download: {
      label: 'Speech model',
      model: 'ggml.bin',
      index: 1,
      count: 4,
      bytes: 41,
      total: 100,
      state: 'downloading' as const,
    },
    restart: () => {},
  });
  expect(container.querySelector('.bar')?.getAttribute('style')).toMatch(/41%/);
});

// A failed file does not end the run: the daemon carries on to the next one and
// refuses a second run while the first holds the slot. Offering a retry here
// would be a button the daemon answers -32005.
it('offers no retry while the run that failed a file is still going', () => {
  const { queryByRole, container } = render(Blockers, {
    blockers: [],
    download: {
      label: 'Speech model',
      model: 'ggml.bin',
      index: 1,
      count: 4,
      bytes: 41,
      total: 100,
      state: 'failed' as const,
    },
    restart: () => {},
  });
  expect(queryByRole('button', { name: 'Try again' })).toBeNull();
  // The visible line names which file failed; the run itself carries on.
  expect(container.querySelector('.progress')?.textContent).toMatch(/failed/);
});

// `forget()` empties the record when saving is switched off, so an established
// install cannot be told apart from a new one by row count alone. Claiming a
// first run at someone months in is worse than staying quiet.
it('does not claim a first run when the record is off rather than empty', () => {
  const { queryByText } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
    first: false,
  });
  expect(queryByText(/types what you say/)).toBeNull();
});
