import { render, screen } from '@testing-library/svelte';
import { expect, it, vi } from 'vitest';
vi.mock('../lib/tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));
import Earlier from './Earlier.svelte';

const rows = [
  { id: 3, text: 'third', timestamp: '2026-08-27T11:00:00Z' },
  { id: 2, text: 'second', timestamp: '2026-08-27T10:00:00Z' },
];

it('counts what the band does not show', () => {
  render(Earlier, { rows, total: 9 });
  expect(screen.getByRole('button', { name: '7 more in History ›' })).toBeTruthy();
});

it('offers no footer when the band shows everything', () => {
  render(Earlier, { rows, total: 2 });
  expect(screen.queryByRole('button', { name: /more in History/ })).toBeNull();
});
