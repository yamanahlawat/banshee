import { tick } from 'svelte';
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  it,
  vi,
  type MockInstance,
} from 'vitest';
import { stubMatchMedia } from './media.mock';
import { leave, travel, travelTime, watchGroups } from './travel';

const REDUCE = '(prefers-reduced-motion: reduce)';
const SETTLE = 'cubic-bezier(0.16, 1, 0.3, 1)';

let ends: (() => void)[];
const finish = () => ends.forEach((end) => end());
let animate: MockInstance<Element['animate']>;

beforeEach(() => {
  ends = [];
  animate = vi.spyOn(Element.prototype, 'animate').mockImplementation(() => {
    const finished = new Promise<void>((done) => ends.push(done));
    return { finished, cancel: vi.fn() } as unknown as Animation;
  });
});

// The module reads one list for the whole run, as a window does, so each test
// moves that list rather than standing in a new one.
let move: (query: string, matches: boolean) => void;
const reduce = (on: boolean) => move(REDUCE, on);
beforeAll(() => {
  move = stubMatchMedia({ [REDUCE]: false });
});
afterAll(() => vi.unstubAllGlobals());

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function place(el: HTMLElement, top: number): HTMLElement {
  Object.defineProperty(el, 'offsetTop', { configurable: true, get: () => top });
  return el;
}

function row(top: number, ...children: HTMLElement[]): HTMLElement {
  const el = place(document.createElement('div'), top);
  el.className = 'row';
  el.append(...children);
  return el;
}

function voice(key: string, top: number): HTMLElement {
  const el = place(document.createElement('div'), top);
  el.dataset.travel = key;
  return el;
}

/// Voice holds Sky; More voices, below it, holds Bella and Adam. Adam moves up
/// into Voice, so More voices and Bella move down by one row of 30.
function getAdam() {
  const bella = voice('bella', 130);
  const more = row(100, bella, voice('adam', 160));
  const group = row(0, voice('sky', 0));
  document.body.append(group, more);

  travel('adam');

  more.querySelector('[data-travel="adam"]')?.remove();
  const adam = voice('adam', 30);
  group.append(adam);
  place(more, 130);
  place(bella, 160);
  return { adam, more, bella };
}

function played(el: Element) {
  return animate.mock.contexts
    .map((context, i) => ({ context, call: animate.mock.calls[i] }))
    .filter(({ context }) => context === el)
    .map(({ call }) => call);
}

it('carries the row from its old place, and moves the rows that make room on one clock', async () => {
  reduce(false);
  const { adam, more, bella } = getAdam();
  await tick();

  const clock = { duration: travelTime(130), easing: SETTLE };
  expect(played(adam)).toEqual([[{ transform: ['translate(0px, 130px)', 'none'] }, clock]]);
  expect(played(more)).toEqual([[{ transform: ['translate(0px, -30px)', 'none'] }, clock]]);
  expect(played(bella)).toEqual([]);
});

it('keeps the moving row on the ground and above the others until it settles', async () => {
  reduce(false);
  const { adam } = getAdam();
  await tick();

  expect(adam.style.zIndex).toBe('2');
  expect(adam.style.background).toBe('var(--ground)');
  finish();
  await tick();
  expect(adam.style.zIndex).toBe('');
  expect(adam.style.background).toBe('');
});

// It arrives under the pointer that pressed Get, so it would carry that
// button's hover up the list.
it('lets the pointer through the moving row until it settles', async () => {
  reduce(false);
  const { adam } = getAdam();
  await tick();

  expect(adam.style.pointerEvents).toBe('none');
  finish();
  await tick();
  expect(adam.style.pointerEvents).toBe('');
});

it('carries the rows that make room on the ground, over whatever fades where they pass', async () => {
  reduce(false);
  const { more } = getAdam();
  await tick();

  expect(more.style.position).toBe('relative');
  expect(more.style.zIndex).toBe('1');
  expect(more.style.background).toBe('var(--ground)');
  expect(more.style.pointerEvents).toBe('');
  finish();
  await tick();
  expect(more.style.position).toBe('');
  expect(more.style.zIndex).toBe('');
  expect(more.style.background).toBe('');
});

it('starts a second move from where each row is drawn, and stops the first', async () => {
  reduce(false);
  const { adam, more, bella } = getAdam();
  await tick();
  const firstOfMore = animate.mock.contexts.indexOf(more);
  const moreFlight = animate.mock.results[firstOfMore].value as Animation;
  const endFirstOfMore = ends[firstOfMore];

  // Part way: Adam is drawn 65 below his slot, More voices 15 above its own.
  adam.style.transform = 'matrix(1, 0, 0, 1, 0, 65)';
  more.style.transform = 'matrix(1, 0, 0, 1, 0, -15)';
  animate.mockClear();
  travel('adam');
  adam.remove();
  const back = voice('adam', 160);
  more.append(back);
  place(more, 100);
  place(bella, 130);
  await tick();

  expect(moreFlight.cancel).toHaveBeenCalled();
  // Drawn at 95, now at 160, inside a block that starts 15 lower than it ends.
  const clock = { duration: travelTime(80), easing: SETTLE };
  expect(played(back)).toEqual([[{ transform: ['translate(0px, -80px)', 'none'] }, clock]]);
  expect(played(more)).toEqual([[{ transform: ['translate(0px, 15px)', 'none'] }, clock]]);
  expect(played(bella)).toEqual([]);

  // The stopped flight ends, and the one that replaced it keeps its ground.
  endFirstOfMore();
  await tick();
  expect(more.style.background).toBe('var(--ground)');
});

it('fades the row in at its new place, and moves nothing, when motion is reduced', async () => {
  reduce(true);
  const { adam } = getAdam();
  await tick();

  expect(animate.mock.calls).toEqual([[{ opacity: [0, 1] }, { duration: 150, easing: 'linear' }]]);
  expect(animate.mock.contexts).toEqual([adam]);
});

it('plays nothing when no row arrived under the key', async () => {
  reduce(false);
  document.body.append(row(0, voice('sky', 0)));
  travel('adam');
  await tick();

  expect(animate).not.toHaveBeenCalled();
});

it('fades what goes with a move out of the flow, and cuts it when motion is reduced', () => {
  reduce(false);
  const going = place(document.createElement('div'), 80);
  expect(leave(going, true).duration).toBe(110);
  expect(going.style.position).toBe('absolute');
  expect(going.style.top).toBe('80px');
  expect(leave(document.createElement('div'), false).duration ?? 0).toBe(0);

  reduce(true);
  const cut = document.createElement('div');
  expect(leave(cut, true).duration ?? 0).toBe(0);
  expect(cut.style.position).toBe('');
});

// Svelte holds a leaving node inert, and gives the node its own inertness
// back when the leave reverses.
it('puts what was leaving back in the flow when the leave reverses, and lifts it again on the next', () => {
  reduce(false);
  const going = place(document.createElement('div'), 80);
  going.style.width = '50%';
  going.inert = true;
  const fade = leave(going, true);
  fade.tick?.(0.5, 0.5);
  expect(going.style.position).toBe('absolute');

  going.inert = false;
  fade.tick?.(1, 0);
  expect(going.style.position).toBe('');
  expect(going.style.top).toBe('');
  expect(going.style.left).toBe('');
  expect(going.style.width).toBe('50%');

  place(going, 120);
  going.inert = true;
  fade.tick?.(0.9, 0.1);
  expect(going.style.position).toBe('absolute');
  expect(going.style.top).toBe('120px');
});

it('takes longer the farther a row goes, from 200 to 300 ms', () => {
  expect(travelTime(0)).toBe(200);
  expect(travelTime(40)).toBeGreaterThan(200);
  expect(travelTime(120)).toBeGreaterThan(travelTime(40));
  expect(travelTime(2000)).toBe(300);
});

it('names the first row that stands in another group, and none on the first look', () => {
  const moved = watchGroups();
  expect(
    moved([
      { key: 'sky', group: 'voice' },
      { key: 'adam', group: 'more' },
    ]),
  ).toBeNull();
  expect(
    moved([
      { key: 'bella', group: 'voice' },
      { key: 'sky', group: 'more' },
      { key: 'adam', group: 'voice' },
    ]),
  ).toBe('sky');
  expect(
    moved([
      { key: 'sky', group: 'more' },
      { key: 'adam', group: 'voice' },
    ]),
  ).toBeNull();
});

it('moves no row into or out of no group', () => {
  const moved = watchGroups();
  moved([
    { key: 'claude', group: null },
    { key: 'codex', group: 'found' },
  ]);
  expect(
    moved([
      { key: 'claude', group: 'found' },
      { key: 'codex', group: null },
    ]),
  ).toBeNull();
  expect(moved([{ key: 'claude', group: 'connected' }])).toBe('claude');
});
