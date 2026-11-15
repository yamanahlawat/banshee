import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { dictated } from '../lib/history.mock';
import type { HistoryRow } from '../lib/tauri';
import HistoryList from './HistoryList.svelte';
import source from './HistoryList.svelte?raw';

const NOW = new Date(2026, 10, 6, 15, 0);
const at = (daysAgo: number, hour: number, minute = 0) =>
  new Date(2026, 10, 6 - daysAgo, hour, minute).toISOString();

const asked = (
  id: number,
  question: string,
  outcome: HistoryRow['outcome'],
  timestamp: string,
  extra: Partial<HistoryRow> = {},
): HistoryRow => ({
  id,
  kind: 'question',
  question,
  text: null,
  outcome,
  reason: null,
  timestamp,
  client: null,
  audio: null,
  ...extra,
});

const said = (count: number, from = 1): HistoryRow[] =>
  Array.from({ length: count }, (_, i) =>
    dictated(from + count - 1 - i, `Line ${from + count - 1 - i}`, at(0, 14, 0)),
  );

// jsdom lays nothing out, so the scroller's height, its offset and each
// option's height are given here. An answered question stands taller than any
// other row, so a height stored against the wrong id shows in the spacers.
const VIEWPORT = 200;
const TALL = 70;
const SHORT = 30;
// One row can be drawn taller after it is measured, as a rewrap would.
let stretched: string | null = null;
const heightOf = (el: Element) =>
  el.textContent?.includes('You answered') || el.textContent?.startsWith(`${stretched},`)
    ? TALL
    : SHORT;

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
    offsets.set(this, value);
  });
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const height = this.getAttribute('role') === 'option' ? heightOf(this) : 0;
    return { x: 0, y: 0, top: 0, left: 0, right: 0, width: 0, bottom: height, height } as DOMRect;
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  stretched = null;
});

function list(rows: HistoryRow[], props: Partial<Record<string, unknown>> = {}) {
  const choose = vi.fn();
  const open = vi.fn();
  const scrolled = vi.fn();
  const view = render(HistoryList, {
    rows,
    selectedId: rows[0]?.id ?? null,
    choose,
    open,
    play: vi.fn(),
    scrolled,
    follow: false,
    opensOnClick: false,
    query: '',
    now: NOW,
    ...props,
  });
  return { ...view, choose, open, scrolled, box: screen.getByRole('listbox') };
}

const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));

async function scrollTo(box: HTMLElement, top: number) {
  box.scrollTop = top;
  await fireEvent.scroll(box);
  await frame();
  await tick();
}

const option = (text: string) => screen.getByRole('option', { name: new RegExp(`^${text}\\b`) });

// Where an option starts inside the list: the spacers and rows above it.
function offsetOf(target: Element): number {
  let sum = 0;
  for (const child of target.parentElement!.children) {
    if (child === target) return sum;
    sum +=
      child.getAttribute('role') === 'option'
        ? heightOf(child)
        : parseFloat((child as HTMLElement).style.height || '0');
  }
  throw new Error('not in the list');
}

describe('the keys', () => {
  beforeEach(layOut);

  it('moves focus with Up, Down, Home and End, choosing each row, and reaches an unmounted row', async () => {
    const rows = said(50);
    const { choose, box } = list(rows);
    await tick();
    expect(box.querySelector('[aria-posinset="50"]')).toBeNull();

    option('Line 50').focus();
    await fireEvent.keyDown(document.activeElement!, { key: 'End' });
    await waitFor(() => expect(document.activeElement).toBe(option('Line 1')));
    expect(choose).toHaveBeenLastCalledWith(1);
    expect(document.activeElement!.getAttribute('aria-posinset')).toBe('50');

    await fireEvent.keyDown(document.activeElement!, { key: 'Home' });
    await waitFor(() => expect(document.activeElement).toBe(option('Line 50')));
    expect(choose).toHaveBeenLastCalledWith(50);

    await fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    await waitFor(() => expect(document.activeElement).toBe(option('Line 49')));
    expect(choose).toHaveBeenLastCalledWith(49);

    await fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' });
    await waitFor(() => expect(document.activeElement).toBe(option('Line 50')));
    expect(choose).toHaveBeenLastCalledWith(50);
  });

  it('keeps one tab stop, on the focused row', async () => {
    const { box } = list(said(5));
    const stops = () =>
      within(box)
        .getAllByRole('option')
        .filter((one) => one.tabIndex === 0);
    expect(stops()).toEqual([option('Line 5')]);

    option('Line 5').focus();
    await fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    await waitFor(() => expect(stops()).toEqual([option('Line 4')]));
  });

  it('opens the focused row on Return', async () => {
    const { open } = list(said(5));
    option('Line 3').focus();
    await fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    expect(open).toHaveBeenCalledWith(3);
  });

  it('chooses a clicked row, and opens it only where a click opens', async () => {
    const { choose, open, unmount } = list(said(5));
    await fireEvent.click(option('Line 3'));
    expect(choose).toHaveBeenCalledWith(3);
    expect(open).not.toHaveBeenCalled();
    unmount();

    const opening = list(said(5), { opensOnClick: true });
    await fireEvent.click(option('Line 2'));
    expect(opening.open).toHaveBeenCalledWith(2);
  });

  it('puts focus back on the row holding the tab stop', async () => {
    const { component } = list(said(5), { selectedId: 3 });
    await (component as unknown as { focus: () => Promise<void> }).focus();
    expect(document.activeElement).toBe(option('Line 3'));
  });
});

describe('the window', () => {
  beforeEach(layOut);

  it('stores each measured height against its own row, with answered questions in the list', async () => {
    const rows = Array.from({ length: 40 }, (_, i) =>
      i % 3 === 0
        ? asked(40 - i, `Question ${40 - i}`, 'transcribed', at(0, 14), { text: 'Yes' })
        : dictated(40 - i, `Line ${40 - i}`, at(0, 14)),
    );
    const { box } = list(rows);
    await tick();
    // Down a viewport at a time, so every row is mounted and measured once.
    for (let top = 0; top <= 1200; top += VIEWPORT) await scrollTo(box, top);

    const truth = (index: number) =>
      rows.slice(0, index).reduce((sum, row) => sum + (row.text === 'Yes' ? TALL : SHORT), 0);
    const mounted = [...box.querySelectorAll('[role="option"]')];
    expect(mounted.length).toBeLessThan(rows.length);
    for (const one of mounted) {
      expect(offsetOf(one)).toBe(truth(Number(one.getAttribute('aria-posinset')) - 1));
    }
  });

  it('keeps the focused row mounted and focused while a wheel scrolls past it', async () => {
    const { box } = list(said(60));
    await tick();
    const held = option('Line 59');
    held.focus();

    await scrollTo(box, 1200);

    expect(box.querySelector('[aria-posinset="1"]')).toBeNull();
    expect(held.isConnected).toBe(true);
    expect(document.activeElement).toBe(held);
    expect(offsetOf(held)).toBe(SHORT);
    expect(box.querySelectorAll('[role="option"]').length).toBeGreaterThan(1);
  });

  it('holds the rows in view still when a row above them changes height', async () => {
    const { box, rerender } = list(said(60));
    await tick();
    await scrollTo(box, 900);
    const mounted = [...box.querySelectorAll('[role="option"]')].filter(
      (one) => one.getAttribute('aria-posinset') !== '1',
    );
    const above = mounted.find((one) => offsetOf(one) + SHORT <= box.scrollTop)!;
    const inView = mounted.find((one) => offsetOf(one) >= box.scrollTop)!;
    const placed = offsetOf(inView) - box.scrollTop;

    stretched = above.textContent!.split(',')[0];
    await rerender({ now: new Date(NOW.getTime() + 60_000) });

    expect(offsetOf(inView) - box.scrollTop).toBe(placed);
  });

  it('leaves selection, focus and the rows in view in place when rows arrive at the top', async () => {
    const rows = said(30);
    const { box, rerender } = list(rows, { selectedId: 18 });
    await tick();
    await scrollTo(box, 300);
    const focused = option('Line 18');
    focused.focus();
    const inView = box.querySelector<HTMLElement>(`[aria-posinset="${300 / SHORT + 1}"]`)!;
    const placed = offsetOf(inView) - box.scrollTop;

    await rerender({ rows: [...said(2, 31), ...rows] });

    expect(box.scrollTop).toBeGreaterThan(300);
    expect(offsetOf(inView) - box.scrollTop).toBe(placed);
    expect(inView.isConnected).toBe(true);
    expect(document.activeElement).toBe(focused);
    expect(focused.getAttribute('aria-selected')).toBe('true');
    expect(focused.getAttribute('aria-posinset')).toBe('15');
    expect(focused.getAttribute('aria-setsize')).toBe('32');
  });

  it('shows rows that arrive while it sits at its top, and keeps selection and focus', async () => {
    const rows = said(30);
    const { box, rerender } = list(rows, { selectedId: 18 });
    await tick();
    const focused = option('Line 18');
    focused.focus();

    await rerender({ rows: [...said(2, 31), ...rows] });

    expect(box.scrollTop).toBe(0);
    expect(box.querySelector('[aria-posinset="1"]')).toBe(option('Line 32'));
    expect(document.activeElement).toBe(focused);
    expect(focused.getAttribute('aria-selected')).toBe('true');
  });
});

describe('the reveal', () => {
  beforeEach(layOut);

  it.each([
    ['the last row', 'End', 1, 1],
    ['a row midway', 'ArrowDown', 20, 10],
  ])(
    'shows %s, chosen by keyboard, below the top inset when a search keeps it',
    async (_, key, presses, id) => {
      const INSET = 8;
      const { box, choose, rerender } = list(said(30));
      await tick();
      option('Line 30').focus();
      for (let i = 0; i < presses; i++) {
        await fireEvent.keyDown(document.activeElement!, { key });
        await waitFor(() => expect(choose).toHaveBeenCalledTimes(i + 1));
        await tick();
      }
      expect(choose).toHaveBeenLastCalledWith(id);

      await rerender({ selectedId: id, query: 'Line' });
      await tick();

      const row = option(`Line ${id}`);
      const content = offsetOf(box.lastElementChild!) + SHORT;
      // The browser keeps the offset between the list's ends, its insets included.
      const end = INSET + content + INSET - VIEWPORT;
      const top = INSET + offsetOf(row) - Math.max(0, Math.min(box.scrollTop, end));
      expect(top).toBeGreaterThanOrEqual(0);
      expect(top).toBeLessThanOrEqual(VIEWPORT - SHORT);
    },
  );
});

describe('following the newest row', () => {
  beforeEach(layOut);

  it('keeps the newest row in view at the top while it follows', async () => {
    const rows = said(30);
    const { box, rerender } = list(rows, { follow: true });
    await tick();

    await rerender({ rows: [...said(2, 31), ...rows], selectedId: 32 });

    expect(box.scrollTop).toBe(0);
    const first = box.querySelector<HTMLElement>('[aria-posinset="1"]')!;
    expect(first).toBe(option('Line 32'));
    expect(offsetOf(first)).toBe(0);
  });

  it('says when the person scrolls the list off its top', async () => {
    const { box, scrolled } = list(said(30));
    await tick();
    await scrollTo(box, 0);
    expect(scrolled).not.toHaveBeenCalled();
    await scrollTo(box, 300);
    expect(scrolled).toHaveBeenCalled();
  });
});

describe('the rows', () => {
  const kinds = (): HistoryRow[] => [
    dictated(8, 'Open the pull request.', at(0, 14, 32)),
    {
      ...dictated(7, '', at(0, 13)),
      text: null,
      outcome: 'failed',
      reason: 'The server refused the key.',
    },
    asked(6, 'Which branch should I use?', 'transcribed', at(0, 12), { text: 'The release one.' }),
    asked(5, 'Shall I push?', 'no_answer', at(1, 18)),
    asked(4, 'Run the suite?', 'closed', at(1, 17)),
    asked(3, 'Tag it?', 'failed', at(1, 16), { reason: 'The model failed to load.' }),
    asked(2, 'Merge it?', 'unasked', at(3, 9), { reason: 'Banshee is muted.' }),
  ];

  // What a sighted reader sees: the row without its screen-reader text.
  const shown = (el: Element) => {
    const copy = el.cloneNode(true) as Element;
    copy.querySelectorAll('.sr').forEach((one) => one.remove());
    return copy.textContent?.replace(/\s+/g, ' ').trim();
  };
  const textOf = (row: Element) => row.querySelector('.cell > .text') as HTMLElement;
  const railOf = (row: Element) => row.querySelector('.cell > .rail') as HTMLElement;

  it('sets each row as a text column, then a rail hidden from the name', () => {
    list(kinds());
    for (const one of screen.getAllByRole('option')) {
      const cell = one.querySelector('.cell')!;
      expect([...cell.children]).toEqual([textOf(one), railOf(one)]);
      expect(railOf(one).getAttribute('aria-hidden')).toBe('true');
    }
  });

  it("puts the time on the rail and the asking agent's name under it, and reads who asked", () => {
    list([asked(1, 'Shall I push?', 'transcribed', at(0, 12), { text: 'Yes.', client: 'claude' })]);
    const row = screen.getByRole('option', { name: /^claude asked Shall I push\?/ });
    const rail = railOf(row);
    expect([...rail.children].map((one) => [one.className.split(' ')[0], one.textContent])).toEqual(
      [
        ['mono', '12:00'],
        ['client', 'claude'],
      ],
    );
    expect(rail.querySelector('.time')).toBe(rail.firstElementChild);
    expect(shown(row)).not.toMatch(/asked/);
  });

  it('shows only the time on the rail for a row with no client, and reads "An agent asked"', () => {
    list([asked(1, 'Shall I push?', 'transcribed', at(0, 12), { text: 'Yes.' })]);
    const row = screen.getByRole('option', { name: /^An agent asked Shall I push\?/ });
    expect(shown(railOf(row))).toBe('12:00');
    expect(row.querySelector('.client')).toBeNull();
    expect(shown(row)).not.toMatch(/asked|an agent/i);
  });

  it('shows only the time on the rail of a dictation', () => {
    list([dictated(1, 'Open the pull request.', at(0, 12))]);
    const row = option('Open the pull request');
    expect(shown(railOf(row))).toBe('12:00');
    expect(row.querySelector('.client')).toBeNull();
  });

  it('keeps the time whole above a long client name, which takes its own element', () => {
    const client = 'c'.repeat(80);
    list([asked(1, 'Shall I push?', 'no_answer', at(0, 12), { client })]);
    const rail = railOf(option(`${client} asked Shall I push`));
    expect(rail.querySelector('.time')?.textContent).toBe('12:00');
    expect(rail.querySelector('.client')?.textContent).toBe(client);
    expect(rail.querySelector('.client')?.contains(rail.querySelector('.time'))).toBe(false);
  });

  it("puts a flag in the answer's place, under the question and hidden from the name", () => {
    list(kinds());
    const flags: [string, string][] = [
      ['An agent asked Shall I push', 'No answer heard'],
      ['An agent asked Run the suite', 'Closed before you answered'],
      ['An agent asked Tag it', 'Not transcribed'],
      ['An agent asked Merge it', 'Not asked aloud'],
      ['Transcription failed', 'The server refused the key.'],
    ];
    for (const [row, flag] of flags) {
      const text = textOf(option(row));
      expect([...text.children].map(shown)).toEqual([
        row.replace('An agent asked ', '') + (row.startsWith('An agent') ? '?' : ''),
        flag,
      ]);
      expect(text.lastElementChild!.classList.contains('flag')).toBe(true);
      expect(text.lastElementChild!.getAttribute('aria-hidden')).toBe('true');
      expect(railOf(option(row)).querySelector('.flag')).toBeNull();
    }
    expect(textOf(option('An agent asked Which branch')).querySelector('.flag')).toBeNull();
    expect(textOf(option('Open the pull request')).querySelector('.flag')).toBeNull();
  });

  it('names each option by its text, its day and time, and its flag', () => {
    list(kinds());
    for (const one of screen.getAllByRole('option')) {
      expect(one.hasAttribute('aria-label')).toBe(false);
    }
    screen.getByRole('option', { name: 'Open the pull request., Today at 14:32' });
    screen.getByRole('option', {
      name: 'Transcription failed, Today at 13:00, The server refused the key.',
    });
    screen.getByRole('option', {
      name: 'An agent asked Which branch should I use? You answered The release one., Today at 12:00',
    });
    screen.getByRole('option', {
      name: 'An agent asked Shall I push?, Yesterday at 18:00, No answer heard',
    });
    screen.getByRole('option', {
      name: 'An agent asked Run the suite?, Yesterday at 17:00, Closed before you answered',
    });
    screen.getByRole('option', {
      name: 'An agent asked Tag it?, Yesterday at 16:00, Not transcribed',
    });
    screen.getByRole('option', {
      name: 'An agent asked Merge it?, 3 November at 09:00, Not asked aloud',
    });
  });

  it('counts the whole set on every option and selects only the chosen row', () => {
    list(kinds(), { selectedId: 5 });
    const options = screen.getAllByRole('option');
    expect(options.map((one) => one.getAttribute('aria-posinset'))).toEqual([
      '1',
      '2',
      '3',
      '4',
      '5',
      '6',
      '7',
    ]);
    expect(options.every((one) => one.getAttribute('aria-setsize') === '7')).toBe(true);
    expect(options.filter((one) => one.getAttribute('aria-selected') === 'true')).toEqual([
      options[3],
    ]);
  });

  it('labels the first row of each day, hidden from the name', () => {
    list(kinds());
    const label = (row: string, day: string) => within(option(row)).queryByText(day);
    expect(label('Open the pull request', 'Today')?.closest('[aria-hidden="true"]')).toBeTruthy();
    // The selection fill and the focus ring sit on the cell, so the label stays outside it.
    expect(label('Open the pull request', 'Today')?.closest('.cell')).toBeNull();
    expect(label('Transcription failed', 'Today')).toBeNull();
    expect(label('An agent asked Shall I push', 'Yesterday')).toBeTruthy();
    expect(label('An agent asked Run the suite', 'Yesterday')).toBeNull();
    expect(label('An agent asked Merge it', '3 November')).toBeTruthy();
  });

  it('puts the date in each row and drops the day labels under a search', () => {
    list(kinds(), { query: 'a' });
    expect(within(option('Transcription failed')).getByText('Today')).toBeTruthy();
    expect(within(option('An agent asked Shall I push')).getByText('Yesterday')).toBeTruthy();
    expect(within(option('An agent asked Run the suite')).getByText('Yesterday')).toBeTruthy();
  });

  it('marks what the search matched in each row, and keeps the names whole', () => {
    const { container } = list(kinds(), { query: 'the' });
    const marks = [...container.querySelectorAll('mark')].map((mark) => mark.textContent);
    expect(marks).toEqual(['the', 'The', 'the']);
    screen.getByRole('option', { name: 'Open the pull request., Today at 14:32' });
    screen.getByRole('option', {
      name: 'An agent asked Which branch should I use? You answered The release one., Today at 12:00',
    });
    screen.getByRole('option', { name: /^An agent asked Run the suite\?/ });
  });

  it('marks nothing with no search', () => {
    const { container } = list(kinds());
    expect(container.querySelector('mark')).toBeNull();
  });

  it('holds no button in an option, and axe passes', async () => {
    const { container } = list(kinds());
    for (const one of screen.getAllByRole('option')) {
      expect(within(one).queryByRole('button')).toBeNull();
    }
    const results = await axe.run(container, {
      rules: { 'color-contrast': { enabled: false } },
    });
    expect(results.violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`)).toEqual([]);
  });
});

describe('contrast', () => {
  it('draws a selected failed row in ink', () => {
    expect(source).toMatch(
      /\.row\[aria-selected='true'\] \.failed[^{]*\{[^}]*color:\s*var\(--ink\)/,
    );
  });

  it("draws a selected row's rail, which carries its time, and its flag in ink", () => {
    for (const part of ['rail', 'flag']) {
      expect(source).toMatch(
        new RegExp(
          `\\.row\\[aria-selected='true'\\] \\.${part}[^{]*\\{[^}]*color:\\s*var\\(--ink\\)`,
        ),
      );
    }
  });

  it("cuts a long name on the rail with an ellipsis, at the rail's fixed width", () => {
    expect(source).toMatch(/\n\s*\.client\s*\{[^}]*text-overflow:\s*ellipsis/);
    expect(source).toMatch(/\n\s*\.client\s*\{[^}]*white-space:\s*nowrap/);
    const cut = source.match(/\n\s*\.client\s*\{[^}]*max-width:\s*(\d+px)/)?.[1];
    expect(cut).toBeDefined();
    expect(source).toMatch(new RegExp(`\\n\\s*\\.rail\\s*\\{[^}]*min-width:\\s*${cut};`));
  });

  it('keeps a date whole on the rail under a search', () => {
    expect(source).toMatch(/\n\s*\.date\s*\{[^}]*white-space:\s*nowrap/);
  });

  // The time takes the rail's dim, and turns with the rail on a selected row.
  it('draws the clock time in the dim rail and leaves the flag in the accent', () => {
    expect(source).toMatch(/<div class="rail[^"]*"[^>]*>[\s\S]*?class="mono time"/);
    expect(source).toMatch(/\n\s*\.rail\s*\{[^}]*color:\s*var\(--dim\)/);
    expect(source).not.toMatch(/\.time[^{]*\{[^}]*color:/);
    expect(source).toMatch(/\n\s*\.flag\s*\{[^}]*color:\s*var\(--accent\)/);
  });
});
