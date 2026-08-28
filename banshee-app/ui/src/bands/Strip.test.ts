import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it } from 'vitest';
import ready from '../fixtures/ready.json';
import { daemon, empty, reduceStatus } from '../lib/daemon';
import { open } from '../lib/jobs';
import Strip from './Strip.svelte';

const values = { Microphone: 'MacBook Pro', Hotkey: 'Right Option', Voice: 'Sky' };

beforeEach(() => {
  daemon.set(reduceStatus(empty(), ready));
  open.set(null);
});

it('names every job and the value it holds', () => {
  render(Strip, { values });
  expect(screen.getByRole('button', { name: /Microphone/ })).toBeTruthy();
  expect(screen.getByText('Right Option')).toBeTruthy();
});

it('opens one job at a time', async () => {
  render(Strip, { values });
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));
  expect(screen.getByRole('button', { name: /Microphone/ }).getAttribute('aria-expanded')).toBe('true');
  await fireEvent.click(screen.getByRole('button', { name: /Hotkey/ }));
  expect(screen.getByRole('button', { name: /Microphone/ }).getAttribute('aria-expanded')).toBe('false');
  expect(screen.getByRole('button', { name: /Hotkey/ }).getAttribute('aria-expanded')).toBe('true');
});

it('closes the job a second press names again', async () => {
  render(Strip, { values });
  const row = screen.getByRole('button', { name: /Voice/ });
  await fireEvent.click(row);
  await fireEvent.click(row);
  expect(row.getAttribute('aria-expanded')).toBe('false');
});

it('shuts a row whose value its owner withheld', () => {
  render(Strip, { values: { ...values, Hotkey: '' } });
  expect(screen.queryByText('Right Option')).toBeNull();
  expect(screen.getByRole('button', { name: /Hotkey/ })).toHaveProperty('disabled', true);
});

it('never reports a row open when it has no panel at all', () => {
  render(Strip, { values });
  const setup = screen.getByRole('button', { name: /Setup/ });
  expect(setup.getAttribute('aria-expanded')).toBeNull();
  expect(setup).toHaveProperty('disabled', true);
});

it('reports a row shut, not absent, when its panel has no value yet', () => {
  // Agents and More settings are openable, but this fixture gives them no
  // value, so they stay silent until App.svelte computes one.
  render(Strip, { values });
  for (const name of [/More settings/, /Agents/]) {
    const row = screen.getByRole('button', { name });
    expect(row.getAttribute('aria-expanded')).toBe('false');
    expect(row).toHaveProperty('disabled', true);
  }
});

it('keeps an open job closable after its value goes away', async () => {
  const { rerender } = render(Strip, { values });
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));
  await rerender({ values: { ...values, Microphone: '' } });

  const row = screen.getByRole('button', { name: /Microphone/ });
  expect(row).toHaveProperty('disabled', false);
  await fireEvent.click(row);
  expect(row.getAttribute('aria-expanded')).toBe('false');
});
