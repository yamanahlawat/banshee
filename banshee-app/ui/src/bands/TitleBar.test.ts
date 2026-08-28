import { render, screen } from '@testing-library/svelte';
import { beforeEach, expect, it } from 'vitest';
import ready from '../fixtures/ready.json';
import { daemon, empty, reduceStatus } from '../lib/daemon';
import TitleBar from './TitleBar.svelte';

beforeEach(() => daemon.set(empty()));

it('names the hotkey the daemon reports', () => {
  daemon.set(reduceStatus(empty(), ready));
  render(TitleBar);
  expect(screen.getByText('Press Right Command to start and stop')).toBeTruthy();
});

it('says nothing about a hotkey before the daemon answers', () => {
  render(TitleBar);
  expect(screen.queryByText(/to start and stop/)).toBeNull();
  expect(screen.queryByText('No hotkey set')).toBeNull();
});

it('says so when the daemon answers with no hotkey', () => {
  const audio = { ...(ready.config.audio as Record<string, unknown>) };
  delete audio.hotkey;
  daemon.set(reduceStatus(empty(), { ...ready, config: { ...ready.config, audio } }));
  render(TitleBar);
  expect(screen.getByText('No hotkey set')).toBeTruthy();
});

it('never tells the user to press a key the daemon has not bound', () => {
  const pendingKey = reduceStatus(empty(), {
    ...ready,
    pending: ['audio.hotkey'],
    config: { ...ready.config, audio: { ...ready.config.audio, hotkey: 'Ctrl+Cmd+M' } },
  });
  daemon.set(pendingKey);
  render(TitleBar);
  expect(screen.getByText('Restart Banshee to use Ctrl + Cmd + M')).toBeTruthy();
  expect(screen.queryByText(/to start and stop/)).toBeNull();
});
