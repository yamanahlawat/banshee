import { tick } from 'svelte';
import type { TransitionConfig } from 'svelte/transition';

/// The mark's settle curve.
const SETTLE = 'cubic-bezier(0.16, 1, 0.3, 1)';

/// A Row moves as one block, and a row marked `data-travel` moves on its own;
/// its value is the key a moved row keeps from one group to the next.
const MOVES = '.row, [data-travel]';

const STILL: TransitionConfig = { duration: 0 };

type Spot = { x: number; y: number };

let motion: MediaQueryList | undefined;

/// Read at each move, so a change to the setting holds from the next one.
export function reduced(): boolean {
  motion ??= window.matchMedia?.('(prefers-reduced-motion: reduce)');
  return motion?.matches ?? false;
}

export function travelTime(distance: number): number {
  return Math.min(300, 200 + distance * 0.45);
}

/// The rows in flight, so a second move can stop them and start from where
/// they are drawn.
const flights = new WeakMap<HTMLElement, Animation>();

// Offsets ignore transforms: this is where the layout puts a row.
function spot(el: HTMLElement): Spot {
  let x = 0;
  let y = 0;
  for (let at: HTMLElement | null = el; at; at = at.offsetParent as HTMLElement | null) {
    x += at.offsetLeft;
    y += at.offsetTop;
  }
  return { x, y };
}

/// Where a row is drawn: its layout place, carried by its own flight and by
/// the flight of every block it rides in.
function drawn(el: HTMLElement): Spot {
  const at = spot(el);
  for (let on: HTMLElement | null = el; on; on = on.parentElement) {
    if (!flights.has(on)) continue;
    const matrix = /matrix\(([^)]+)\)/.exec(getComputedStyle(on).transform);
    if (!matrix) continue;
    const values = matrix[1].split(',').map(Number);
    at.x += values[4];
    at.y += values[5];
  }
  return at;
}

function measure() {
  const byElement = new Map<HTMLElement, Spot>();
  const byKey = new Map<string, Spot>();
  for (const el of document.querySelectorAll<HTMLElement>(MOVES)) {
    const at = drawn(el);
    byElement.set(el, at);
    if (el.dataset.travel) byKey.set(el.dataset.travel, at);
  }
  return { byElement, byKey };
}

function play(first: ReturnType<typeof measure>, key: string) {
  const all = [...document.querySelectorAll<HTMLElement>(MOVES)];
  const moved = all.find((el) => el.dataset.travel === key && !first.byElement.has(el));
  if (!moved) return;
  if (reduced()) {
    moved.animate({ opacity: [0, 1] }, { duration: 150, easing: 'linear' });
    return;
  }

  const shifts = new Map<HTMLElement, Spot>();
  for (const el of all) {
    const was = first.byElement.get(el) ?? (el === moved ? first.byKey.get(key) : undefined);
    if (!was) continue;
    const now = spot(el);
    shifts.set(el, { x: was.x - now.x, y: was.y - now.y });
  }
  // A row inside a block that moves rides with it, so it moves only by the
  // difference.
  const own = (el: HTMLElement): Spot => {
    const shift = shifts.get(el) as Spot;
    for (let at = el.parentElement; at; at = at.parentElement) {
      const carried = shifts.get(at);
      if (carried) return { x: shift.x - carried.x, y: shift.y - carried.y };
    }
    return shift;
  };
  const moves = [...shifts.keys()]
    .map((el) => ({ el, by: own(el) }))
    .filter(({ by }) => Math.abs(by.x) >= 0.5 || Math.abs(by.y) >= 0.5);
  const duration = travelTime(Math.max(0, ...moves.map(({ by }) => Math.hypot(by.x, by.y))));

  for (const el of all) flights.get(el)?.cancel();
  for (const { el, by } of moves) {
    const animation = el.animate(
      { transform: [`translate(${by.x}px, ${by.y}px)`, 'none'] },
      { duration, easing: SETTLE },
    );
    flights.set(el, animation);
    // Every row in flight is on the ground, so no words show through it: not
    // those of the rows it passes, nor of what fades where it passes. The row
    // that moved passes over all of them, and arrives under the pointer that
    // sent it, so it takes no hover on the way.
    const arriving = el === moved;
    Object.assign(el.style, {
      position: 'relative',
      zIndex: arriving ? '2' : '1',
      background: 'var(--ground)',
      pointerEvents: arriving ? 'none' : '',
    });
    const settled = () => {
      if (flights.get(el) !== animation) return;
      flights.delete(el);
      Object.assign(el.style, { position: '', zIndex: '', background: '', pointerEvents: '' });
    };
    animation.finished.then(settled, settled);
  }
}

/// Remembers which group each keyed row stood in, and names the first row that
/// stands in another group now. A row in no group neither leaves nor arrives.
export function watchGroups(): (rows: { key: string; group: string | null }[]) => string | null {
  let stood = new Map<string, string | null>();
  return (rows) => {
    const moved = rows.find(({ key, group }) => {
      const was = stood.get(key);
      return was != null && group !== null && was !== group;
    });
    stood = new Map(rows.map(({ key, group }) => [key, group]));
    return moved?.key ?? null;
  };
}

/// Called before the DOM takes the change that moves the row keyed `key` to
/// another group. The rows are read now and moved once the change has landed.
export function travel(key: string): void {
  const first = measure();
  void tick().then(() => play(first, key));
}

/// What goes with a move, such as a group the move left empty, fades where it
/// stood. It leaves the flow at once, so the rows that make room are read at
/// their final places.
export function leave(node: HTMLElement, leaves: boolean): TransitionConfig {
  if (!leaves || reduced()) return STILL;
  const { position, top, left, width } = node.style;
  let lifted = false;
  const lift = () => {
    const style = getComputedStyle(node);
    Object.assign(node.style, {
      position: 'absolute',
      top: `${node.offsetTop - parseFloat(style.marginTop)}px`,
      left: `${node.offsetLeft - parseFloat(style.marginLeft)}px`,
      width: `${node.offsetWidth}px`,
    });
    lifted = true;
  };
  lift();
  return {
    duration: 110,
    css: (t) => `opacity: ${t};`,
    // Svelte holds a leaving node inert, and gives it its own inertness back
    // when the outro reverses. A later outro of the same node reuses this
    // config rather than calling `leave` again.
    tick: () => {
      if (node.inert === lifted) return;
      if (node.inert) {
        lift();
      } else {
        Object.assign(node.style, { position, top, left, width });
        lifted = false;
      }
    },
  };
}
