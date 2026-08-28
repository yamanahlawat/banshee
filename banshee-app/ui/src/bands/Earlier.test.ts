import { fireEvent, render, screen } from '@testing-library/svelte';
import { expect, it, vi } from 'vitest';
vi.mock('../lib/tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));
import { get } from 'svelte/store';
import { open } from '../lib/jobs';
import Earlier from './Earlier.svelte';

const rows = [
  { id: 3, text: 'third', timestamp: '2026-08-27T11:00:00Z' },
  { id: 2, text: 'second', timestamp: '2026-08-27T10:00:00Z' },
];

it('counts what the band does not show', () => {
  render(Earlier, { rows, more: 7, history: 'some' });
  expect(screen.getByRole('button', { name: '7 more in History ›' })).toBeTruthy();
});

it('opens History when the count line is pressed', async () => {
  open.set(null);
  render(Earlier, { rows, more: 7, history: 'some' });
  await fireEvent.click(screen.getByRole('button', { name: '7 more in History ›' }));
  expect(get(open)).toBe('More settings');
});

it('offers no footer when the band shows everything', () => {
  render(Earlier, { rows, more: 0, history: 'some' });
  expect(screen.queryByRole('button', { name: /more in History/ })).toBeNull();
});

it('says the day is empty rather than leaving a count with no rows above it', () => {
  render(Earlier, { rows: [], more: 1836, history: 'some' });
  expect(screen.getByText('Nothing said today')).toBeTruthy();
  // The count is still the way into a history that is not empty.
  expect(screen.getByRole('button', { name: '1836 more in History ›' })).toBeTruthy();
});

it('keeps the never-recorded line distinct from the nothing-today line', () => {
  render(Earlier, { rows: [], more: 0, history: 'empty' });
  expect(screen.getByText('Nothing saved yet')).toBeTruthy();
});

it('does not call a machine empty when its one dictation sits in the pad', () => {
  // The pad holds the only row, so the band is told a count of zero.
  render(Earlier, { rows: [], more: 0, history: 'some' });
  expect(screen.queryByText('Nothing saved yet')).toBeNull();
  expect(screen.getByText('Nothing said today')).toBeTruthy();
});

it('says nothing about an emptiness it has not read', () => {
  render(Earlier, { rows: [], more: 0, history: 'unread' });
  expect(screen.queryByText('Nothing saved yet')).toBeNull();
  expect(screen.getByText('History unread')).toBeTruthy();
});
