import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { setSetting } from '../lib/tauri';
import { daemon, type Status } from '../lib/daemon';
import { table } from '../lib/settings';
import { commonTable } from '../lib/tauri.mock';
import FeedbackPage from './FeedbackPage.svelte';

const MAC_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
const LINUX_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15';

function ready(feedback?: Record<string, unknown>) {
  const config: Record<string, unknown> = {};
  if (feedback) config.feedback = feedback;
  const status = { running: true, config } as unknown as Status;
  daemon.update((state) => ({ ...state, status, pending: new Set<string>() }));
}

// A daemon set to visual differs by platform, so the page needs its own user
// agent per test rather than the one jsdom happens to report.
function renderPage(options: { userAgent?: string; feedback?: Record<string, unknown> } = {}) {
  if (options.userAgent !== undefined) {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(options.userAgent);
  }
  ready(options.feedback);
  return render(FeedbackPage);
}

beforeEach(() => {
  table.set(commonTable);
  vi.mocked(setSetting).mockReset();
  vi.mocked(setSetting).mockResolvedValue(undefined as never);
  ready();
});

afterEach(() => {
  vi.restoreAllMocks();
});

it('maps a daemon reporting visual to Both, and says it was set from the command line', () => {
  ready({ mode: 'visual' });
  const page = render(FeedbackPage);
  const chosen = page.getByRole('radio', { name: 'Both' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
  expect(
    page.getByText(
      'Set to visual outside this window. Without the on-screen figure, Banshee plays every sound.',
    ),
  ).toBeTruthy();
});

it('points the feedback radiogroup at its note', () => {
  ready({ mode: 'sound' });
  const page = render(FeedbackPage);
  const group = page.getByRole('radiogroup', { name: 'Feedback' });
  const note = page.getByText(
    'A short sound when Banshee starts and stops listening, and when it fails.',
  );
  expect(group.getAttribute('aria-describedby')).toBe(note.id);
});

it('offers On screen on macOS and writes visual', async () => {
  const page = renderPage({ userAgent: MAC_UA });
  await fireEvent.click(page.getByRole('radio', { name: 'On screen' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('feedback.mode', 'visual'));
});

it('selects On screen for a daemon set to visual, with no sounds and the VoiceOver advice', () => {
  const page = renderPage({ userAgent: MAC_UA, feedback: { mode: 'visual' } });
  const chosen = page.getByRole('radio', { name: 'On screen' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
  expect(page.getByText(/shows what Banshee does, with no sounds/)).toBeTruthy();
  expect(page.getByText(/An agent's questions are still spoken aloud/)).toBeTruthy();
  expect(page.getByText(/Using VoiceOver\? Choose Both/)).toBeTruthy();
});

it('offers no On screen where no figure draws', () => {
  const page = renderPage({ userAgent: LINUX_UA, feedback: { mode: 'visual' } });
  expect(page.queryByRole('radio', { name: 'On screen' })).toBeNull();
  const chosen = page.getByRole('radio', { name: 'Both' });
  expect(chosen.getAttribute('aria-checked')).toBe('true');
});

it('names no figure in the Both and Off notes on Linux', () => {
  const both = renderPage({ userAgent: LINUX_UA, feedback: { mode: 'both' } });
  expect(both.getByText('Every sound.')).toBeTruthy();
  both.unmount();

  const off = renderPage({ userAgent: LINUX_UA, feedback: { mode: 'none' } });
  expect(off.getByText("No sound. An agent's questions are still spoken aloud.")).toBeTruthy();
});

it('writes feedback.mode', async () => {
  const page = render(FeedbackPage);
  await fireEvent.click(page.getByRole('radio', { name: 'Sound' }));
  await waitFor(() => expect(setSetting).toHaveBeenCalledWith('feedback.mode', 'sound'));
});
