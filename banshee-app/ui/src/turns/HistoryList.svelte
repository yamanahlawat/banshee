<script lang="ts">
  import { afterUpdate, onMount } from 'svelte';

  import { land } from '../lib/focus';

  import { dayLabel, rowFlag } from '../lib/history';
  import type { HistoryRow } from '../lib/tauri';
  import { formatTime, sameLocalDay, toDate } from '../lib/time';
  import { plan, segments, span, type Segment } from '../lib/vlist';

  type Id = HistoryRow['id'];

  export let rows: HistoryRow[];
  export let selectedId: Id | null;
  export let choose: (id: Id) => void;
  export let open: (id: Id) => void;
  /// Called when the person moves the list off its top.
  export let scrolled: () => void;
  /// Rows that arrive stay in view at the top, rather than above it.
  export let follow: boolean;
  /// A click opens the row as Return does, where the detail is not beside the list.
  export let opensOnClick: boolean;
  /// The trimmed search the rows answer. Under one, each row shows its date and
  /// the day labels go.
  export let query: string;
  export let now: Date;

  const OVERSCAN = 3;
  // Not measured in a window: worked from this file's CSS for a two-line row.
  // 20px of padding, two 15px lines at 1.35, the 2px gap and a 13px meta line at
  // 1.45. Measured heights replace it as rows mount.
  const ESTIMATE = 81;

  let scroller: HTMLElement;
  let scrollTop = 0;
  // Null until mounted, so the first render does not mount every row.
  let viewport: number | null = null;
  let scrollFrame = 0;
  // Nothing reads this for reactivity: `replan` is called outright when a
  // measurement lands.
  const known = new Map<Id, number>();
  // The option holding focus, which stays mounted wherever the scroll goes.
  let focused: Id | null = null;
  let lead: Id | null = null;
  // Added to the scroller's offset once the DOM holds the rows that arrived.
  let shift = 0;
  let parts: Segment[] = [];
  // Empty at mount, so a list that mounts under a search shows its selection.
  let searched = '';
  let revealing = false;

  const heightAt = (i: number) => known.get(rows[i]?.id) ?? null;
  const heightAbove = (index: number) => span(heightAt, ESTIMATE, 0, index);

  function tabStop(list: HistoryRow[], holding: Id | null, chosen: Id | null): number {
    const at = (id: Id | null) => list.findIndex((row) => row.id === id);
    const held = at(holding);
    return held !== -1 ? held : Math.max(0, at(chosen));
  }

  // The list sets `overflow-anchor: none`, so rows arriving above are its own
  // business: the offset grows by their height and the rows in view stay put.
  // At its top the list shows a new row instead, but rows a cleared search
  // brings back still keep the chosen row in its place. A new search shows the
  // selected row, which is its first match unless the chosen row still matches.
  function arrive(list: HistoryRow[], search: string) {
    const was = lead;
    lead = list[0]?.id ?? null;
    const refiltered = search !== searched;
    if (refiltered) {
      searched = search;
      revealing = search !== '';
    }
    const added = list.findIndex((row) => row.id === was);
    if (added <= 0 || follow || (scrollTop === 0 && !refiltered)) return;
    const grown = heightAbove(added);
    scrollTop += grown;
    shift += grown;
  }

  function replan(list: HistoryRow[], top: number, height: number | null, kept: number) {
    let next: Segment[];
    if (height === null) next = [];
    // With no height to window against, as in jsdom, every row mounts.
    else if (height === 0) next = [{ rows: [0, list.length] }];
    else {
      const win = plan(list.length, heightAt, ESTIMATE, top, height, OVERSCAN);
      next = segments(win, kept, list.length, heightAt, ESTIMATE);
    }
    // Compared rather than assigned: most scroll ticks leave the window where
    // it was, and a fresh array would re-render and re-measure every row.
    if (JSON.stringify(next) !== JSON.stringify(parts)) parts = next;
  }

  // One keyed list for spacers and rows alike, so a row passing between the
  // window and the kept slot moves no node, and focus stays on it.
  type Entry = { key: string; pad: number } | { key: string; index: number };
  function entries(layout: Segment[], list: HistoryRow[]): Entry[] {
    const out: Entry[] = [];
    layout.forEach((part, k) => {
      if ('spacer' in part) {
        const next = layout[k + 1];
        const below = next && 'rows' in next ? list[next.rows[0]]?.id : 'end';
        out.push({ key: `pad-${below}`, pad: part.spacer });
        return;
      }
      for (let i = part.rows[0]; i < part.rows[1]; i++) {
        out.push({ key: `row-${list[i].id}`, index: i });
      }
    });
    return out;
  }

  $: dated = query !== '';
  $: arrive(rows, query);
  $: stop = tabStop(rows, focused, selectedId);
  $: replan(rows, scrollTop, viewport, stop);
  $: items = entries(parts, rows);

  function gauge() {
    if (!scroller || !viewport) return;
    const firstShown = plan(rows.length, heightAt, ESTIMATE, scrollTop, viewport, 0).start;
    let above = 0;
    let moved = false;
    for (const node of scroller.querySelectorAll<HTMLElement>('[role="option"]')) {
      const index = Number(node.getAttribute('aria-posinset')) - 1;
      const id = rows[index].id;
      const h = node.getBoundingClientRect().height;
      if (!(h > 0) || known.get(id) === h) continue;
      if (index < firstShown) above += h - (heightAt(index) ?? ESTIMATE);
      known.set(id, h);
      moved = true;
    }
    if (above !== 0) {
      scroller.scrollTop += above;
      scrollTop = scroller.scrollTop;
    }
    if (moved) replan(rows, scrollTop, viewport, stop);
  }

  function settle() {
    if (!scroller) return;
    // A list that has just mounted is empty until it plans its parts, and an
    // empty list scrolls nowhere.
    if (revealing && parts.length > 0) {
      // The selected row goes to the top, as far as the list scrolls.
      const at = rows.findIndex((row) => row.id === selectedId);
      revealing = false;
      shift = 0;
      scroller.scrollTop = heightAbove(Math.max(0, at));
      scrollTop = scroller.scrollTop;
    } else if (shift !== 0) {
      scroller.scrollTop += shift;
      shift = 0;
    }
    gauge();
  }

  // Native scroll events outrun the display on a trackpad fling. One read a
  // frame is all a paint can use.
  function onScroll() {
    if (scrollFrame) return;
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = 0;
      scrollTop = scroller.scrollTop;
      if (scrollTop > 0) scrolled();
    });
  }

  function readViewport() {
    scrollTop = scroller.scrollTop;
    viewport = scroller.clientHeight;
    // A width change rewraps every row without moving the window.
    gauge();
  }

  const optionAt = (index: number) =>
    scroller.querySelector<HTMLElement>(`[aria-posinset="${index + 1}"]`);

  export function focus() {
    return land(() => optionAt(stop));
  }

  async function move(to: number) {
    const id = rows[to].id;
    focused = id;
    choose(id);
    await land(() => optionAt(to));
  }

  function onKey(event: KeyboardEvent, index: number) {
    const last = rows.length - 1;
    const to: Record<string, number> = {
      ArrowDown: Math.min(index + 1, last),
      ArrowUp: Math.max(index - 1, 0),
      Home: 0,
      End: last,
    };
    if (event.key === 'Enter') {
      event.preventDefault();
      open(rows[index].id);
    } else if (event.key in to) {
      event.preventDefault();
      move(to[event.key]);
    }
  }

  function onFocusOut(event: FocusEvent) {
    if (!scroller.contains(event.relatedTarget as Node | null)) focused = null;
  }

  onMount(() => {
    readViewport();
    // Until the faces land the widths are the fallback's, so a row measured
    // at first paint may not hold once the real cut arrives.
    document.fonts?.ready?.then(gauge);
    if (typeof ResizeObserver === 'undefined') return () => cancelAnimationFrame(scrollFrame);
    const watcher = new ResizeObserver(readViewport);
    watcher.observe(scroller);
    return () => {
      cancelAnimationFrame(scrollFrame);
      watcher.disconnect();
    };
  });
  afterUpdate(settle);

  const firstOfDay = (index: number) =>
    index === 0 || !sameLocalDay(toDate(rows[index - 1].timestamp), toDate(rows[index].timestamp));
</script>

<div
  class="list"
  role="listbox"
  aria-label="History, newest first"
  bind:this={scroller}
  on:scroll={onScroll}
  on:focusout={onFocusOut}
>
  {#each items as item (item.key)}
    {#if 'pad' in item}
      <div aria-hidden="true" style:height="{item.pad}px"></div>
    {:else}
      {@const row = rows[item.index]}
      {@const day = dayLabel(row.timestamp, now)}
      {@const time = formatTime(row.timestamp)}
      {@const flag = rowFlag(row)}
      <div
        class="row"
        role="option"
        aria-selected={row.id === selectedId}
        aria-posinset={item.index + 1}
        aria-setsize={rows.length}
        tabindex={item.index === stop ? 0 : -1}
        on:focus={() => (focused = row.id)}
        on:click={() => (opensOnClick ? open : choose)(row.id)}
        on:keydown={(event) => onKey(event, item.index)}
      >
        {#if !dated && firstOfDay(item.index)}
          <span class="day" aria-hidden="true">{day}</span>
        {/if}
        <!-- Inside the last paragraph: beside it, the option's name would gain a
             space before the comma. -->
        {#snippet when()}<span class="sr">{`, ${day} at ${time}`}{flag ? `, ${flag}` : ''}</span
          >{/snippet}
        {#if row.kind === 'dictation' && row.outcome === 'failed'}
          <p class="words failed">Transcription failed{@render when()}</p>
        {:else if row.kind === 'dictation'}
          <p class="words user">{row.text ?? ''}{@render when()}</p>
        {:else if row.outcome === 'transcribed'}
          <p class="words agent one"><span class="sr">Agent asked</span> {row.question ?? ''}</p>
          <p class="words user one">
            <span class="sr">You answered</span>
            {row.text ?? ''}{@render when()}
          </p>
        {:else}
          <p class="words agent">
            <span class="sr">Agent asked</span>
            {row.question ?? ''}{@render when()}
          </p>
        {/if}
        <span class="meta" aria-hidden="true">
          {#if dated}<span>{day}</span>{/if}
          <span class="mono time">{time}</span>
          {#if flag}<span class="flag">{flag}</span>{/if}
        </span>
      </div>
    {/if}
  {/each}
</div>

<style>
  .list {
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    overflow-anchor: none;
  }

  .row {
    position: relative;
    padding: 10px 12px;
    border-radius: var(--radius-row);
    cursor: default;
  }

  .row[aria-selected='true'] {
    background: var(--hilite);
  }

  .row:focus-visible {
    outline-offset: -2px;
  }

  .day {
    display: block;
    margin-bottom: 4px;
    font-size: 13px;
    line-height: 1.45;
    font-weight: 600;
    color: var(--dim);
  }

  .words {
    margin: 0;
    font-size: 15px;
    line-height: 1.35;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .one {
    -webkit-line-clamp: 1;
    line-clamp: 1;
  }

  .user {
    font-variation-settings:
      'wght' var(--cut-user-weight),
      'wdth' var(--cut-user-width);
  }

  .agent {
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
  }

  .failed {
    color: var(--accent);
    font-weight: 600;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    column-gap: 8px;
    margin-top: 2px;
    font-size: 13px;
    line-height: 1.45;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    color: var(--dim);
  }

  .time,
  .flag {
    color: var(--accent);
  }

  .flag {
    font-variation-settings:
      'wght' 600,
      'wdth' var(--cut-agent-width);
  }

  .row[aria-selected='true'] .meta,
  .row[aria-selected='true'] .time,
  .row[aria-selected='true'] .flag {
    color: var(--ink);
  }
</style>
