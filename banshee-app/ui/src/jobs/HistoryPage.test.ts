import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { dictated } from '../lib/history.mock';
import type { HistoryRow } from '../lib/tauri';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import { copyText } from '../lib/tauri';
import { clearSearch, table, view } from '../lib/history';
import { forgetCopy } from '../lib/copy';
import { onMac } from '../lib/keys';
import { LIST_AND_DETAIL_FROM, SIDEBAR_FOLDS_BELOW } from '../lib/layout';
import { stubMatchMedia } from '../lib/media.mock';
import HistoryPage from './HistoryPage.svelte';

const NOW = new Date(2026, 10, 6, 15, 0);
const at = (hour: number) => new Date(2026, 10, 6, hour, 0).toISOString();

const asked = (id: number, question: string, row: Partial<HistoryRow> = {}): HistoryRow => ({
  id,
  kind: 'question',
  question,
  text: null,
  outcome: 'no_answer',
  reason: null,
  timestamp: at(8),
  ...row,
});

// Newest first, as the table holds them.
const ROWS: HistoryRow[] = [
  asked(3, 'Which branch?', {
    text: 'The release branch.',
    outcome: 'transcribed',
    timestamp: at(11),
  }),
  dictated(2, 'Open the pull request.', at(10)),
  asked(1, 'Shall I push?', { timestamp: at(9) }),
];

const WIDE = `(min-width: ${LIST_AND_DETAIL_FROM}px)`;
const SIDEBAR = `(min-width: ${SIDEBAR_FOLDS_BELOW}px)`;

function show(rows: HistoryRow[], props: Record<string, unknown> = {}) {
  table.set({ rows, total: rows.length, loaded: true, saving: true });
  const go = vi.fn();
  const view = render(HistoryPage, {
    saving: true,
    alerting: false,
    downloading: false,
    agentless: false,
    waiting: false,
    host: null,
    compositor: false,
    hotkey: 'Right Command',
    verb: 'Tap',
    go,
    ...props,
  });
  return { ...view, go };
}

const option = (text: string) => screen.getByRole('option', { name: new RegExp(text) });
const detail = () => screen.queryByRole('region', { name: 'Selected row' });
const notices = () => screen.queryByRole('region', { name: 'Notices' });
// Svelte sets `inert` as a property, and jsdom has no attribute behind it.
function inert(element: Element): boolean {
  for (let at: Element | null = element; at; at = at.parentElement) {
    if ((at as HTMLElement & { inert?: boolean }).inert) return true;
  }
  return false;
}
const copyChord = { key: 'c', [onMac() ? 'metaKey' : 'ctrlKey']: true };

async function arrive(row: HistoryRow) {
  table.update((held) => ({ ...held, rows: [row, ...held.rows], total: held.total + 1 }));
  await tick();
}

// jsdom lays nothing out, so the list's height, its offset and each option's
// height are given here, as the list's own tests give them.
const VIEWPORT = 200;
const ROW = 30;
function layOut() {
  const offsets = new WeakMap<Element, number>();
  vi.spyOn(Element.prototype, 'clientHeight', 'get').mockImplementation(function (this: Element) {
    return this.getAttribute('role') === 'listbox' ? VIEWPORT : 0;
  });
  vi.spyOn(Element.prototype, 'scrollTop', 'get').mockImplementation(function (this: Element) {
    return offsets.get(this) ?? 0;
  });
  vi.spyOn(Element.prototype, 'scrollTop', 'set').mockImplementation(function (
    this: Element,
    value: number,
  ) {
    const content = [...this.children].reduce((sum, child) => sum + heightOf(child), 0);
    offsets.set(this, Math.min(Math.max(0, value), Math.max(0, content - VIEWPORT)));
  });
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const height = this.getAttribute('role') === 'option' ? ROW : 0;
    return { x: 0, y: 0, top: 0, left: 0, right: 0, width: 0, bottom: height, height } as DOMRect;
  });
}

const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));

async function scrollTo(box: HTMLElement, top: number) {
  box.scrollTop = top;
  await fireEvent.scroll(box);
  await frame();
  await tick();
}

// Where an option starts inside the list: the spacers and rows above it.
function offsetOf(target: Element): number {
  let sum = 0;
  for (const child of target.parentElement!.children) {
    if (child === target) return sum;
    sum += heightOf(child);
  }
  throw new Error('not in the list');
}

function heightOf(child: Element): number {
  return child.getAttribute('role') === 'option'
    ? ROW
    : parseFloat((child as HTMLElement).style.height || '0');
}

// A row the list cannot scroll to the top sits lower, still in view.
function inView(target: Element, box: Element) {
  const top = offsetOf(target) - box.scrollTop;
  expect(top).toBeGreaterThanOrEqual(0);
  expect(top).toBeLessThanOrEqual(VIEWPORT - ROW);
}

async function violations(container: HTMLElement): Promise<string[]> {
  const results = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } });
  return results.violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`);
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  vi.mocked(copyText).mockReset();
  vi.mocked(copyText).mockResolvedValue(undefined);
  view.set({ query: '', chosen: null });
  forgetCopy();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  window.getSelection()?.removeAllRanges();
});

describe('beside the list', () => {
  it('moves the day on at midnight with no new row', async () => {
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
    vi.setSystemTime(new Date(2026, 10, 6, 23, 59));
    show([dictated(1, 'Late.', new Date(2026, 10, 6, 23, 30).toISOString())]);
    expect(within(detail()!).getByRole('heading', { name: /^Today at/ })).toBeTruthy();

    await vi.advanceTimersByTimeAsync(2 * 60_000);
    expect(within(detail()!).getByRole('heading', { name: /^Yesterday at/ })).toBeTruthy();
  });

  it('selects the newest row at open, and Cmd+C copies its answer or text, else its question', async () => {
    show(ROWS);
    expect(option('Which branch').getAttribute('aria-selected')).toBe('true');
    expect(within(detail()!).getByText('The release branch.')).toBeTruthy();

    await fireEvent.keyDown(document.body, copyChord);
    expect(copyText).toHaveBeenLastCalledWith('The release branch.');

    await fireEvent.click(option('Open the pull request'));
    await fireEvent.keyDown(document.body, copyChord);
    expect(copyText).toHaveBeenLastCalledWith('Open the pull request.');

    await fireEvent.click(option('Shall I push'));
    await fireEvent.keyDown(document.body, copyChord);
    expect(copyText).toHaveBeenLastCalledWith('Shall I push?');
  });

  it('leaves Cmd+C to a text selection or a text field', async () => {
    show(ROWS);
    const range = document.createRange();
    range.selectNodeContents(within(detail()!).getByText('The release branch.'));
    window.getSelection()!.addRange(range);
    await fireEvent.keyDown(document.body, copyChord);
    window.getSelection()!.removeAllRanges();

    const field = document.createElement('input');
    document.body.append(field);
    field.focus();
    await fireEvent.keyDown(field, copyChord);
    field.remove();

    expect(copyText).not.toHaveBeenCalled();
  });

  it('follows the newest row until the person chooses one, then holds it', async () => {
    show(ROWS);
    await arrive(dictated(4, 'A fourth.', at(12)));
    expect(option('A fourth').getAttribute('aria-selected')).toBe('true');

    await fireEvent.click(option('Open the pull request'));
    await arrive(dictated(5, 'A fifth.', at(13)));
    expect(option('Open the pull request').getAttribute('aria-selected')).toBe('true');
    expect(option('A fifth').getAttribute('aria-selected')).toBe('false');
    expect(within(detail()!).getByText('Open the pull request.')).toBeTruthy();
  });

  it('holds the selection once the person scrolls the list', async () => {
    layOut();
    const rows = Array.from({ length: 30 }, (_, i) => dictated(30 - i, `Line ${30 - i}`, at(9)));
    show(rows);
    await tick();
    await scrollTo(screen.getByRole('listbox'), 300);

    await arrive(dictated(31, 'Line 31', at(12)));
    expect(option('^Line 30,').getAttribute('aria-selected')).toBe('true');
    expect(within(detail()!).getByText('Line 30')).toBeTruthy();
  });

  it('holds the focused row selected as new rows arrive', async () => {
    show(ROWS);
    const held = option('Which branch');
    held.focus();
    await fireEvent.focusIn(held);

    await arrive(dictated(4, 'A fourth.', at(12)));
    expect(held.getAttribute('aria-selected')).toBe('true');
    expect(within(detail()!).getByText('The release branch.')).toBeTruthy();
    await fireEvent.keyDown(held, copyChord);
    expect(copyText).toHaveBeenLastCalledWith('The release branch.');
  });

  it('holds the row whose Copy button has focus as new rows arrive', async () => {
    show(ROWS);
    const button = within(detail()!).getByRole('button', { name: 'Copy answer' });
    button.focus();
    await fireEvent.focusIn(button);

    await arrive(dictated(4, 'A fourth.', at(12)));
    expect(option('Which branch').getAttribute('aria-selected')).toBe('true');
    expect(within(detail()!).getByText('The release branch.')).toBeTruthy();
    expect(document.activeElement).toBe(button);
  });

  it('holds the row whose Copy button is pressed without focus as new rows arrive', async () => {
    show(ROWS);
    const button = within(detail()!).getByRole('button', { name: 'Copy answer' });
    await fireEvent.pointerDown(button);

    await arrive(dictated(4, 'A fourth.', at(12)));
    expect(option('Which branch').getAttribute('aria-selected')).toBe('true');
    expect(within(detail()!).getByText('The release branch.')).toBeTruthy();
  });

  it('moves focus to the detail heading on Return, and Escape brings it back to the row', async () => {
    show(ROWS);
    option('Open the pull request').focus();
    await fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    await waitFor(() =>
      expect(document.activeElement).toBe(within(detail()!).getByRole('heading')),
    );

    await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(document.activeElement).toBe(option('Open the pull request')));
  });
});

describe('a search', () => {
  it('filters the list, dates each row, and says when nothing matches', async () => {
    show(ROWS);
    view.update((now) => ({ ...now, query: 'pull' }));
    await tick();
    expect(screen.getAllByRole('option')).toEqual([option('Open the pull request')]);
    expect(within(option('Open the pull request')).getByText('Today')).toBeTruthy();

    view.update((now) => ({ ...now, query: 'nothing like it' }));
    await tick();
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(screen.getByText('No match')).toBeTruthy();
    expect(screen.getByText('Nothing said so far contains “nothing like it”.')).toBeTruthy();
    expect(detail()).toBeNull();
  });

  it('starts the list at its top when the selection is the first match', async () => {
    layOut();
    const rows = Array.from({ length: 30 }, (_, i) => dictated(30 - i, `Line ${30 - i}`, at(9)));
    show(rows);
    await tick();
    const box = screen.getByRole('listbox');
    await scrollTo(box, 300);

    view.update((now) => ({ ...now, query: 'Line 1' }));
    await tick();
    await tick();
    expect(box.scrollTop).toBe(0);
    const first = box.querySelector('[aria-posinset="1"]')!;
    expect(first.getAttribute('aria-selected')).toBe('true');
  });

  it('scrolls a selection that still matches into view when the search changes', async () => {
    layOut();
    const rows = Array.from({ length: 30 }, (_, i) => dictated(30 - i, `Line ${30 - i}`, at(9)));
    show(rows);
    await tick();
    view.update((now) => ({ ...now, chosen: 3 }));
    await tick();
    const box = screen.getByRole('listbox');
    expect(box.scrollTop).toBe(0);

    view.update((now) => ({ ...now, query: 'Line' }));
    await tick();
    await tick();
    const kept = option('^Line 3,');
    expect(kept.getAttribute('aria-selected')).toBe('true');
    inView(kept, box);
  });

  it('shows a selection that still matches when the list returns from no match', async () => {
    layOut();
    const rows = Array.from({ length: 30 }, (_, i) => dictated(30 - i, `Line ${30 - i}`, at(9)));
    show(rows);
    await tick();
    view.update((now) => ({ ...now, chosen: 3 }));
    await tick();
    view.update((now) => ({ ...now, query: 'nothing like it' }));
    await tick();

    view.update((now) => ({ ...now, query: 'Line' }));
    await tick();
    await tick();
    const kept = option('^Line 3,');
    expect(kept.getAttribute('aria-selected')).toBe('true');
    inView(kept, screen.getByRole('listbox'));
  });

  it('keeps the selection and its place in view when it is cleared', async () => {
    layOut();
    const rows = Array.from({ length: 30 }, (_, i) =>
      dictated(30 - i, i === 20 ? 'the odd one' : `Line ${30 - i}`, at(9)),
    );
    show(rows);
    await tick();
    view.update((now) => ({ ...now, query: 'odd' }));
    await tick();
    const box = screen.getByRole('listbox');
    expect(box.scrollTop).toBe(0);

    clearSearch();
    await tick();
    await tick();

    const kept = option('the odd one');
    expect(kept.getAttribute('aria-selected')).toBe('true');
    expect(offsetOf(kept) - box.scrollTop).toBe(0);
  });
});

describe('below the width the list and detail share', () => {
  let widen: (query: string, matches: boolean) => void;
  beforeEach(() => {
    widen = stubMatchMedia({ [WIDE]: false, [SIDEBAR]: true });
  });

  it('opens no row until one is chosen, and a click opens it', async () => {
    show(ROWS);
    expect(detail()).toBeNull();
    expect(option('Which branch').getAttribute('aria-selected')).toBe('true');

    await fireEvent.click(option('Open the pull request'));
    await waitFor(() =>
      expect(document.activeElement).toBe(within(detail()!).getByRole('heading')),
    );
    expect(inert(screen.getByRole('listbox'))).toBe(true);

    await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(document.activeElement).toBe(option('Open the pull request')));
    expect(detail()).toBeNull();
    expect(inert(screen.getByRole('listbox'))).toBe(false);
  });

  it('drops an open row once the selection leaves it, so it never reopens unasked', async () => {
    show(ROWS);
    await fireEvent.click(option('Open the pull request'));
    await waitFor(() => expect(detail()).toBeTruthy());

    view.update((now) => ({ ...now, query: 'branch' }));
    await tick();
    expect(detail()).toBeNull();
    clearSearch();
    await tick();
    expect(option('Which branch').getAttribute('aria-selected')).toBe('true');

    option('Which branch').focus();
    await fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    await waitFor(() => expect(document.activeElement).toBe(option('Open the pull request')));
    expect(detail()).toBeNull();
    expect(inert(screen.getByRole('listbox'))).toBe(false);
  });

  it('opens a row on Return, and Escape returns to it', async () => {
    show(ROWS);
    option('Shall I push').focus();
    await fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    await waitFor(() =>
      expect(document.activeElement).toBe(within(detail()!).getByRole('heading')),
    );
    expect(within(detail()!).getByText('Shall I push?')).toBeTruthy();

    await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(document.activeElement).toBe(option('Shall I push')));
    expect(detail()).toBeNull();
  });

  it('keeps focus on a visible element, in the pane that held it, across the width', async () => {
    show(ROWS);
    widen(WIDE, true);
    await tick();
    option('Open the pull request').focus();
    await fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    const heading = await waitFor(() => within(detail()!).getByRole('heading'));
    expect(document.activeElement).toBe(heading);

    widen(WIDE, false);
    await tick();
    expect(detail()).toBeTruthy();
    expect(document.activeElement).toBe(heading);
    expect(inert(screen.getByRole('listbox'))).toBe(true);

    widen(WIDE, true);
    await tick();
    expect(document.activeElement).toBe(heading);

    option('Shall I push').focus();
    widen(WIDE, false);
    await tick();
    expect(detail()).toBeNull();
    expect(document.activeElement).toBe(option('Shall I push'));
    expect(inert(screen.getByRole('listbox'))).toBe(false);
  });

  it('keeps a row open that focus reached from wide, as new rows arrive', async () => {
    show(ROWS);
    widen(WIDE, true);
    await tick();
    const button = within(detail()!).getByRole('button', { name: 'Copy answer' });
    button.focus();
    await fireEvent.focusIn(button);

    widen(WIDE, false);
    await tick();
    expect(detail()).toBeTruthy();

    await arrive(dictated(4, 'A fourth.', at(12)));
    expect(detail()).toBeTruthy();
    expect(within(detail()!).getByText('The release branch.')).toBeTruthy();
    expect(document.activeElement).toBe(button);
  });

  it('carries no accessibility violations with the list or a row open', async () => {
    const { container } = show(ROWS);
    expect(await violations(container)).toEqual([]);
    await fireEvent.click(option('Which branch'));
    await waitFor(() => expect(detail()).toBeTruthy());
    expect(await violations(container)).toEqual([]);
  });
});

describe('an empty History', () => {
  it('says how to start, with a key', () => {
    show([]);
    expect(screen.getByRole('heading', { name: 'Nothing here yet' })).toBeTruthy();
    expect(
      screen.getByText(
        'Tap Right Command and talk in any app. Your dictations show here, and so do the questions agents ask you.',
      ),
    ).toBeTruthy();
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(detail()).toBeNull();
  });

  it('opens Hotkey when no key is bound', async () => {
    const { go } = show([], { hotkey: '' });
    expect(screen.getByText('No key is bound, so nothing starts dictation.')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Open Hotkey' }));
    expect(go).toHaveBeenCalledWith('Hotkey');
  });

  it('names the compositor where it binds the key', () => {
    show([], { compositor: true });
    expect(
      screen.getByText(
        'Your compositor starts dictation here. Your dictations show here, and so do the questions agents ask you.',
      ),
    ).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Open Hotkey' })).toBeNull();
  });

  it('says Save history is off, and opens it', async () => {
    const { go } = show([], { saving: false });
    expect(
      screen.getByRole('heading', { name: 'Save history is off, so nothing new is kept.' }),
    ).toBeTruthy();
    expect(screen.queryByText('Nothing here yet')).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Open Saved history' }));
    expect(go).toHaveBeenCalledWith('Saved history');
  });

  it('asks for an agent when none is connected', async () => {
    const { go } = show([], { agentless: true });
    expect(
      screen.getByText(
        'Tap Right Command and talk in any app. Your dictations show here. No agent can speak to you yet. Connect one, and its questions show here too.',
      ),
    ).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Connect an agent' }));
    expect(go).toHaveBeenCalledWith('Agents');
  });

  it('keeps Open Hotkey the one action when no key is bound and no agent is connected', () => {
    show([], { hotkey: '', agentless: true });
    expect(screen.getByRole('button', { name: 'Open Hotkey' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Connect an agent' })).toBeNull();
  });

  it('says nothing before the table is read', () => {
    table.set({ rows: [], total: 0, loaded: false, saving: null });
    render(HistoryPage, {
      saving: true,
      alerting: false,
      downloading: false,
      agentless: false,
      waiting: false,
      host: null,
      compositor: false,
      hotkey: 'Right Command',
      verb: 'Tap',
      go: vi.fn(),
    });
    expect(screen.queryByText('Nothing here yet')).toBeNull();
  });

  it('stays under a notice, saying what the page is for and leaving the action to the notice', () => {
    show([], { alerting: true, agentless: true });
    expect(notices()).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Nothing here yet' })).toBeTruthy();
    expect(
      screen.getByText('Your dictations show here, and so do the questions agents ask you.'),
    ).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('still says no key is bound under a notice, and leaves the action to it', () => {
    show([], { alerting: true, hotkey: '' });
    expect(screen.getByText('No key is bound, so nothing starts dictation.')).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('asks a compositor user for an agent when none is connected', async () => {
    const { go } = show([], { compositor: true, agentless: true });
    expect(
      screen.getByText(
        'Your compositor starts dictation here. Your dictations show here. No agent can speak to you yet. Connect one, and its questions show here too.',
      ),
    ).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Connect an agent' }));
    expect(go).toHaveBeenCalledWith('Agents');
  });

  it('gives way to a download, which is the page while it runs', () => {
    show([], { alerting: true, downloading: true });
    expect(screen.queryByText('Nothing here yet')).toBeNull();
  });

  it('carries no accessibility violations', async () => {
    const { container } = show([], { hotkey: '' });
    expect(await violations(container)).toEqual([]);
  });
});

describe('the notices above the list', () => {
  it('draws no stack when nothing needs saying', () => {
    show(ROWS);
    expect(notices()).toBeNull();
  });

  it('says an agent is waiting, with the key that answers', () => {
    show(ROWS, { waiting: true });
    const stack = notices()!;
    expect(
      within(stack).getByText('An agent asked a question and is waiting for your answer.'),
    ).toBeTruthy();
    // Only the key name is a machine value, so only it takes the mono face.
    const how = within(stack).getByText(
      (_, node) => node?.tagName === 'P' && node.textContent === 'Tap Right Command to answer.',
    );
    expect(within(how).getByText('Right Command').classList.contains('mono')).toBe(true);
    expect(stack.contains(screen.getByRole('listbox'))).toBe(false);
  });

  it('sends the answer to the compositor binding where it holds the key', () => {
    show(ROWS, { waiting: true, compositor: true });
    expect(
      within(notices()!).getByText("Press your compositor's Banshee binding to answer."),
    ).toBeTruthy();
  });

  it('says Save history is off while rows stay, and opens it', async () => {
    const { go } = show(ROWS, { saving: false });
    const stack = notices()!;
    expect(within(stack).getByText('Save history is off, so nothing new is kept.')).toBeTruthy();
    await fireEvent.click(within(stack).getByRole('button', { name: 'Open Saved history' }));
    expect(go).toHaveBeenCalledWith('Saved history');
    expect(screen.getByRole('listbox')).toBeTruthy();
  });

  it('names the host at the top of the stack while the sidebar is folded', () => {
    stubMatchMedia({ [WIDE]: false, [SIDEBAR]: false });
    show(ROWS, { host: 'api.openai.com', waiting: true });
    const stack = notices()!;
    const line = within(stack).getByText(
      (_, node) => node?.textContent === 'Audio goes to api.openai.com',
    );
    expect(stack.firstElementChild).toBe(line);
  });

  it('names no host while the sidebar shows it', () => {
    show(ROWS, { host: 'api.openai.com' });
    expect(notices()).toBeNull();
  });

  it('carries no accessibility violations with every notice up', async () => {
    const { container } = show(ROWS, { waiting: true, saving: false, alerting: true });
    expect(await violations(container)).toEqual([]);
  });
});
