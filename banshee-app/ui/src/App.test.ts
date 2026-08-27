import { fireEvent, render, screen } from '@testing-library/svelte';
import { expect, it, vi } from 'vitest';
import ready from './fixtures/ready.json';
vi.mock('./lib/tauri', () => ({
  status: vi.fn().mockResolvedValue(ready),
  history: vi.fn().mockResolvedValue([{ id: 1, text: 'Yes, open the pull request.', timestamp: '2026-08-26T13:47:00Z' }]),
  listen: vi.fn().mockResolvedValue(() => {}),
  copyText: vi.fn(),
}));
import App from './App.svelte';
it('opens on Ready with the latest dictation and one live region', async () => {
  render(App);
  expect(await screen.findByText('Ready')).toBeTruthy();
  expect(await screen.findByText('Yes, open the pull request.')).toBeTruthy();
  expect(document.querySelectorAll('[aria-live]').length).toBe(1);
});
it('speaks a copy confirmation through the live region', async () => {
  render(App);
  const copyButton = await screen.findByRole('button', { name: 'Copy' });
  await fireEvent.click(copyButton);
  const region = document.querySelector('[aria-live]');
  expect(region?.textContent).toContain('Copied');
});
