import { readable, type Readable } from 'svelte/store';

export const SIDEBAR_FOLDS_BELOW = 700;
export const LIST_AND_DETAIL_FROM = 860;

function query(minWidth: number): Readable<boolean> {
  return readable(true, (set) => {
    if (typeof window.matchMedia !== 'function') {
      set(true);
      return;
    }
    const list = window.matchMedia(`(min-width: ${minWidth}px)`);
    const update = () => set(list.matches);
    update();
    list.addEventListener('change', update);
    return () => list.removeEventListener('change', update);
  });
}

export const sidebarShown = query(SIDEBAR_FOLDS_BELOW);
export const listAndDetail = query(LIST_AND_DETAIL_FROM);
