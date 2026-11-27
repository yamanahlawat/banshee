import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { daemon, type Status } from '../lib/daemon';
import { WAITING_SAYS } from '../lib/copy';
import { forgetRefusals, table, write } from '../lib/settings';
import { setSetting } from '../lib/tauri';
import { commonTable } from '../lib/tauri.mock';
import { expectInOrder } from '../lib/order.mock';
import Row from './Row.svelte';
import HotkeyPage from '../jobs/HotkeyPage.svelte';
import source from './Row.svelte?raw';
import failure from './Failure.svelte?raw';
import tokens from '../lib/tokens.css?raw';

beforeEach(() => {
  table.set(commonTable);
  forgetRefusals();
  vi.mocked(setSetting).mockResolvedValue([]);
  const status = { running: true, config: { feedback: { mode: 'sound' } } } as unknown as Status;
  daemon.update((state) => ({ ...state, status, pending: new Set<string>() }));
});

it('keeps the note after the control and linked to it', () => {
  const page = render(HotkeyPage);
  const group = page.getByRole('radiogroup', { name: 'Feedback' });
  const note = page.getByText('A failure plays a sound too.');
  expect(group.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(group.getAttribute('aria-describedby')).toBe(note.id);
});

// jsdom lays nothing out, so the rule is read from the component's own style.
it('keeps the label column for every row', () => {
  const columns = [...source.matchAll(/grid-template-columns:\s*([^;]+);/g)].map(
    ([, value]) => value,
  );
  expect(columns.length).toBeGreaterThan(0);
  for (const value of columns) expect(value).toMatch(/^var\(--label-column\) /);
  expect(tokens).toMatch(/--label-column: 112px;/);
});

// A failure is the provider group's, so it starts where the group's controls do.
it('starts a failure on the control edge', () => {
  expect(failure).toMatch(/margin-left: calc\(var\(--label-column\) \+ var\(--label-gap\)\);/);
});

it('says a wait the daemon does not report in place of the key line', () => {
  daemon.update((state) => ({ ...state, pending: new Set(['stt.remote.model']) }));
  const page = render(Row, {
    name: 'Model',
    setting: 'stt.remote.model',
    line: 'Download it first.',
  });
  expect(page.getByText('Download it first.').classList.contains('pending')).toBe(true);
  expect(page.queryByText(WAITING_SAYS)).toBeNull();
});

const STOPPED = { code: -32000, message: 'the daemon closed the socket', transport: true };

it('says a refused write on its row, after the note and the pending line', async () => {
  daemon.update((state) => ({ ...state, pending: new Set(['feedback.mode']) }));
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  const page = render(HotkeyPage);
  const group = page.getByRole('radiogroup', { name: 'Feedback' });
  await fireEvent.click(page.getByRole('radio', { name: 'Off' }));

  const refusal = await waitFor(() => page.getByText('Not changed. Banshee is not running.'));
  expectInOrder(
    group,
    page.getByText('A failure plays a sound too.'),
    page.getByText(WAITING_SAYS),
    refusal,
  );
  // The window's own announcement says it once; a live note would say it twice.
  expect(refusal.closest('[aria-live]')).toBeNull();
  const described = group.getAttribute('aria-describedby')?.split(' ') ?? [];
  expect(refusal.id).not.toBe('');
  expect(described).toContain(refusal.id);
  expect(described).toContain(page.getByText('A failure plays a sound too.').id);
});

// The model list and the key row write a key whose pending line another row
// already carries, or that has none.
it('shows its refusal without its pending line when the pending line is off', async () => {
  daemon.update((state) => ({ ...state, pending: new Set(['stt.model']) }));
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  await write('stt.model', 'x', () => {});
  const page = render(Row, { name: 'Model', setting: 'stt.model', line: null });
  expect(page.getByText('Not changed. Banshee is not running.')).toBeTruthy();
  expect(page.container.querySelector('.pending')).toBeNull();
});

it('keeps its pending line and leaves the refusal to another row when the refusal is off', async () => {
  daemon.update((state) => ({ ...state, pending: new Set(['feedback.mode']) }));
  vi.mocked(setSetting).mockRejectedValue(STOPPED);
  await write('feedback.mode', 'x', () => {});
  const page = render(Row, { name: 'Feedback', setting: 'feedback.mode', refusal: false });
  expect(page.getByText(WAITING_SAYS)).toBeTruthy();
  expect(page.queryByText('Not changed. Banshee is not running.')).toBeNull();
});
