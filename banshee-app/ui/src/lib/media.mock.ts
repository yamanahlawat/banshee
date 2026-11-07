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
