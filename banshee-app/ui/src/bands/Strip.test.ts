import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it } from 'vitest';
import ready from '../fixtures/ready.json';
import notRunning from '../fixtures/not-running.json';
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

it('strips the values and shuts the rows when the daemon is not running', () => {
  daemon.set({ ...reduceStatus(empty(), notRunning), down: 'not running' });
  render(Strip, { values });
  expect(screen.queryByText('Right Option')).toBeNull();
  expect(screen.getByRole('button', { name: /Microphone/ })).toHaveProperty('disabled', true);
});

it('never reports a row open when nothing opens behind it', () => {
  render(Strip, { values });
  for (const name of [/More settings/, /Setup/, /Agents/]) {
    const row = screen.getByRole('button', { name });
    expect(row.getAttribute('aria-expanded')).toBeNull();
    expect(row).toHaveProperty('disabled', true);
  }
});

it('keeps an open job closable after the daemon stops', async () => {
  render(Strip, { values });
  await fireEvent.click(screen.getByRole('button', { name: /Microphone/ }));
  daemon.set({ ...reduceStatus(empty(), notRunning), down: 'not running' });

  const row = screen.getByRole('button', { name: /Microphone/ });
  expect(row).toHaveProperty('disabled', false);
  await fireEvent.click(row);
  expect(row.getAttribute('aria-expanded')).toBe('false');
});
