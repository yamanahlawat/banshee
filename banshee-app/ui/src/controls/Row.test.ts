import { render } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { daemon, type Status } from '../lib/daemon';
import { table } from '../lib/settings';
import { commonTable } from '../lib/tauri.mock';
import FeedbackPage from '../jobs/FeedbackPage.svelte';

beforeEach(() => {
  table.set(commonTable);
  const status = { running: true, config: { feedback: { mode: 'sound' } } } as unknown as Status;
  daemon.update((state) => ({ ...state, status, pending: new Set<string>() }));
});

it('keeps the note after the control and linked to it', () => {
  const page = render(FeedbackPage);
  const group = page.getByRole('radiogroup', { name: 'Feedback' });
  const note = page.getByText('A failure plays a sound too.');
  expect(group.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(group.getAttribute('aria-describedby')).toBe(note.id);
});
