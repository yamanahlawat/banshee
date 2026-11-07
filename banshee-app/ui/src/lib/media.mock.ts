import { vi } from 'vitest';

type Listener = () => void;

/// Stands in for `matchMedia`, answering each query from `matching` and false
/// for any other. Returns the way to move one query and tell its listeners.
export function stubMatchMedia(matching: Record<string, boolean>) {
  const lists = new Map<string, { matches: boolean; listeners: Set<Listener> }>();
  vi.stubGlobal('matchMedia', (q: string) => {
    const list = lists.get(q) ?? { matches: matching[q] ?? false, listeners: new Set<Listener>() };
    lists.set(q, list);
    return {
      get matches() {
        return list.matches;
      },
      addEventListener: (_: string, l: Listener) => list.listeners.add(l),
      removeEventListener: (_: string, l: Listener) => list.listeners.delete(l),
    };
  });
  return (q: string, matches: boolean) => {
    const list = lists.get(q)!;
    list.matches = matches;
    list.listeners.forEach((l) => l());
  };
}

/// jsdom plays no media and makes no object URLs. Play and pause fire the
/// events a browser fires, load fires none, and each URL made is `blob:clip-{n}`.
export function stubMedia() {
  const load = vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => {});
  const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(function (
    this: HTMLMediaElement,
  ) {
    this.dispatchEvent(new Event('play'));
    return Promise.resolve();
  });
  const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(function (
    this: HTMLMediaElement,
  ) {
    this.dispatchEvent(new Event('pause'));
  });
  const made = { create: URL.createObjectURL, revoke: URL.revokeObjectURL };
  let count = 0;
  URL.createObjectURL = vi.fn(() => `blob:clip-${(count += 1)}`);
  URL.revokeObjectURL = vi.fn();
  return {
    play,
    pause,
    load,
    restore() {
      load.mockRestore();
      play.mockRestore();
      pause.mockRestore();
      URL.createObjectURL = made.create;
      URL.revokeObjectURL = made.revoke;
    },
  };
}
