import { get } from 'svelte/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { stubMatchMedia } from './media.mock';

beforeEach(() => vi.resetModules());
afterEach(() => vi.unstubAllGlobals());

describe('layout stores', () => {
  it('breakpoints are the design values', async () => {
    const m = await import('./layout');
    expect(m.SIDEBAR_FOLDS_BELOW).toBe(700);
    expect(m.LIST_AND_DETAIL_FROM).toBe(860);
  });

  it('sidebarShown follows its media query', async () => {
    const flip = stubMatchMedia({ '(min-width: 700px)': true });
    const { sidebarShown } = await import('./layout');
    const seen: boolean[] = [];
    const stop = sidebarShown.subscribe((v) => seen.push(v));
    flip('(min-width: 700px)', false);
    flip('(min-width: 700px)', true);
    stop();
    expect(seen).toEqual([true, false, true]);
  });

  it('listAndDetail follows its own query, not the sidebar one', async () => {
    const flip = stubMatchMedia({ '(min-width: 700px)': true, '(min-width: 860px)': false });
    const { listAndDetail, sidebarShown } = await import('./layout');
    expect(get(listAndDetail)).toBe(false);
    flip('(min-width: 860px)', true);
    expect(get(listAndDetail)).toBe(true);
    expect(get(sidebarShown)).toBe(true);
  });

  it('both are true where matchMedia is absent', async () => {
    vi.stubGlobal('matchMedia', undefined);
    const { sidebarShown, listAndDetail } = await import('./layout');
    expect(get(sidebarShown)).toBe(true);
    expect(get(listAndDetail)).toBe(true);
  });

  it('is true again once matchMedia is gone, whatever it last answered', async () => {
    stubMatchMedia({});
    const { sidebarShown } = await import('./layout');
    sidebarShown.subscribe(() => {})();
    vi.unstubAllGlobals();
    vi.stubGlobal('matchMedia', undefined);
    expect(get(sidebarShown)).toBe(true);
  });
});
