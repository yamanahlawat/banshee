<script lang="ts">
  import { afterUpdate, onMount } from 'svelte';

  import { plan } from '../lib/vlist';
  import type { HistoryRow } from '../lib/tauri';

  export let rows: HistoryRow[] = [];
  /// Read from the scrolling ancestor, which App owns.
  export let scrollTop = 0;
  export let viewport = 0;
  /// The scrolling ancestor itself. The record does not start at the top of it:
  /// the ledger, the blockers and a waiting turn all stand above, and `plan`
  /// counts from the first row.
  export let container: HTMLElement | null = null;
  export let overscan = 3;
  // Measured in a real window over 500 rows: median 62, from 41 to a 137px
  // lead. Gauged rows replace it as they mount.
  export let estimate = 62;
  let box: HTMLElement;
  // Nothing reads this for reactivity: `replan` is called outright when a
  // measurement lands, and a SvelteMap would suggest the block follows it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const known = new Map<HistoryRow['id'], number>();
  let head = 0;
  let w = { start: 0, end: 0, top: 0, bottom: 0 };

  // Called rather than computed inline: a legacy reactive statement tracks the
  // identifiers it names, and a measurement landing in `known` is invisible to
  // it. Every value the plan reads is a parameter, so both callers state them.
  function replan(
    list: HistoryRow[],
    top: number,
    height: number,
    over: number,
    guess: number,
    offset: number,
  ): void {
    // Also the path jsdom takes, where nothing lays out.
    const next =
      height > 0
        ? plan(
            list.length,
            (i) => known.get(list[i]?.id) ?? null,
            guess,
            top - offset,
            height,
            over,
          )
        : { start: 0, end: list.length, top: 0, bottom: 0 };
    // Compared rather than assigned: overscan means most scroll ticks leave the
    // window where it was, and a fresh object would still re-render and send
    // `gauge` back through a measurement pass over every mounted row.
    if (
      next.start !== w.start ||
      next.end !== w.end ||
      next.top !== w.top ||
      next.bottom !== w.bottom
    ) {
      w = next;
    }
  }

  $: replan(rows, scrollTop, viewport, overscan, estimate, head);

  function measureHead() {
    if (!box || !container) return;
    const next =
      box.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
    if (next !== head) head = next;
  }

  // `offsetHeight` stops at the border box, and a turn carries its spacing as a
  // margin, so a row measured without it leaves the spacers short by that much
  // for every row they stand in for.
  function outerHeight(el: HTMLElement): number {
    const box = getComputedStyle(el);
    return el.offsetHeight + parseFloat(box.marginTop) + parseFloat(box.marginBottom);
  }

  function gauge() {
    measureHead();
    if (!box || !(viewport > 0)) return;
    const ids = rows.slice(w.start, w.end).map((r) => r.id);
    const nodes = box.querySelectorAll(':scope > article.turn');
    // Re-read on the same rows too: a width change rewraps every turn without
    // moving the window, and the heights held would be the old wrapping's.
    let moved = false;
    nodes.forEach((node, k) => {
      const h = outerHeight(node as HTMLElement);
      if (h > 0 && known.get(ids[k]) !== h) {
        known.set(ids[k], h);
        moved = true;
      }
    });
    if (moved) replan(rows, scrollTop, viewport, overscan, estimate, head);
  }

  onMount(() => {
    gauge();
    // Until the faces land the widths are the fallback's, so a row measured
    // at first paint may not hold once the real cut arrives.
    document.fonts?.ready?.then(gauge);
  });
  afterUpdate(gauge);
</script>

<div class="record" bind:this={box}>
  {#if w.top > 0}<div class="pad" aria-hidden="true" style="height: {w.top}px"></div>{/if}
  {#each rows.slice(w.start, w.end) as row, j (row.id)}
    <slot {row} index={w.start + j} />
  {/each}
  {#if w.bottom > 0}<div class="pad" aria-hidden="true" style="height: {w.bottom}px"></div>{/if}
</div>

<style>
  .record {
    min-width: 0;
  }

  .pad {
    width: 100%;
  }
</style>
