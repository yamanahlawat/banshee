import { render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it } from 'vitest';
import permissions from '../fixtures/permissions.json';
import ready from '../fixtures/ready.json';
import recording from '../fixtures/recording.json';
import { daemon, empty, reduceLive, reduceStatus } from '../lib/daemon';
import Scale from './Scale.svelte';

beforeEach(() => daemon.set(empty()));

it('marks the first station that is not clear as the current step', () => {
  daemon.set(reduceStatus(empty(), permissions));
  const { container } = render(Scale);
  const current = container.querySelector('[aria-current="step"]');
  expect(current?.textContent).toContain('Permissions');
  expect(current?.textContent).toContain('blocked');
});

it('reaches Try it when nothing blocks the machine', () => {
  daemon.set(reduceStatus(empty(), ready));
  const { container } = render(Scale);
  expect(container.querySelector('[aria-current="step"]')?.textContent).toContain('Try it');
});

it('speaks every station once, although the eye sees some twice', () => {
  daemon.set(reduceStatus(empty(), ready));
  const { container } = render(Scale);
  // The row below the line repeats the even labels for the eye only.
  const spoken = [...container.querySelectorAll('ol li')].map((li) => li.textContent);
  expect(spoken).toEqual([
    'Running, clear',
    'Microphone, clear',
    'Permissions, clear',
    'Models, clear',
    'Try it, todo',
  ]);
  expect(container.querySelectorAll('[aria-hidden="true"] span').length).toBeGreaterThan(0);
});

it('keeps the live colour off the needle while recording', () => {
  daemon.set(reduceLive(reduceStatus(empty(), ready), recording));
  const { container } = render(Scale);
  expect(container.querySelector('svg g line')?.getAttribute('stroke')).toBe('var(--ink)');
});

it('drops the labels and names the station in compact form', () => {
  daemon.set(reduceStatus(empty(), permissions));
  const { container } = render(Scale, { compact: true });
  expect(container.querySelector('section')?.getAttribute('aria-label')).toBe(
    'Setup progress, Permissions',
  );
  expect(container.querySelector('ol')).toBeNull();
});

it('does not share a landmark name with the fixes band', () => {
  daemon.set(reduceStatus(empty(), permissions));
  const { container } = render(Scale);
  expect(container.querySelector('section')?.getAttribute('aria-label')).toBe('Setup progress');
});
