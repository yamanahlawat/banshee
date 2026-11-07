<script lang="ts">
  import { tick } from 'svelte';
  import { copy } from '../lib/copy';
  import { copyOf, listed, needle, selected, table, view } from '../lib/history';
  import { keysClaimed, onMac } from '../lib/keys';
  import { listAndDetail, sidebarShown } from '../lib/layout';
  import { today } from '../lib/time';
  import { tabWhenScrolls } from '../lib/scrolls';
  import type { Page } from '../lib/nav';
  import type { HistoryRow } from '../lib/tauri';
  import Absence from '../turns/Absence.svelte';
  import HostLine from '../bands/HostLine.svelte';
  import HistoryDetail from '../turns/HistoryDetail.svelte';
  import HistoryList from '../turns/HistoryList.svelte';

  type Id = HistoryRow['id'];

  export let saving: boolean;
  /// The window's own notices fill the `alerts` slot, and say why nothing works.
  export let alerting: boolean;
  /// A download runs, and its box is the page while it does.
  export let downloading: boolean;
  /// The daemon answered and no agent is connected.
  export let agentless: boolean;
  /// An agent asked a question and waits for the answer.
  export let waiting: boolean;
  /// Where audio goes while a remote listener is in use.
  export let host: string | null;
  /// The compositor binds the key, so Banshee binds none.
  export let compositor: boolean;
  /// The bound key, empty when none is.
  export let hotkey: string;
  export let verb: 'Hold' | 'Tap';
  export let go: (page: Page) => void;
  /// True while an open row takes the list's place, below the width they share.
  export let reading = false;

  const mac = onMac();
  const SHOWS = 'Your dictations show here, and so do the questions agents ask you.';

  let list: HistoryList | undefined;
  let detail: HistoryDetail | undefined;
  let detailPane: HTMLElement | undefined;
  /// The row the detail shows in the list's place, below the width they share.
  let opened: Id | null = null;

  $: rows = $table.rows;
  $: row = selected($listed, $view.chosen);
  $: wide = $listAndDetail;
  // Before `reading`, which reads what it decides. Runs before the DOM swaps
  // panes, so focus is read while the pane holding it still exists.
  $: refit(wide);
  // A row stays open only while it is the selected one.
  $: if (opened !== null && row?.id !== opened) opened = null;
  $: reading = !wide && opened !== null;
  // The sidebar names the host while it shows.
  $: shownHost = $sidebarShown ? null : host;
  $: savingOff = !saving && rows.length > 0;
  $: stacked = shownHost !== null || alerting || waiting || savingOff;
  $: says = compositor
    ? 'Your compositor starts dictation here.'
    : hotkey
      ? `${verb} ${hotkey} and talk in any app.`
      : 'No key is bound, so nothing starts dictation.';
  $: keyless = !compositor && !hotkey;
  // A notice above holds the one action, and may say why the key does nothing.
  $: empty = alerting
    ? { detail: keyless ? says : SHOWS }
    : keyless
      ? { detail: says, action: 'Open Hotkey', to: 'Hotkey' as Page }
      : agentless
        ? {
            detail: `${says} Your dictations show here. No agent can speak to you yet. Connect one, and its questions show here too.`,
            action: 'Connect an agent',
            to: 'Agents' as Page,
          }
        : { detail: `${says} ${SHOWS}` };

  function choose(id: Id) {
    view.update((now) => ({ ...now, chosen: id }));
  }

  // Scrolling the list, or moving focus or the pointer into the list or the
  // detail, is a choice too: the row in view, or the one touched, stays
  // selected as rows arrive. WebKit on macOS focuses no pressed button.
  function hold() {
    if ($view.chosen === null && row) choose(row.id);
  }

  async function open(id: Id) {
    choose(id);
    opened = id;
    await tick();
    detail?.focus();
  }

  export async function back() {
    opened = null;
    await list?.focus();
  }

  export function focusDetail() {
    detail?.focus();
  }

  function refit(wide: boolean) {
    if (!wide) opened = detailPane?.contains(document.activeElement) && row ? row.id : null;
  }

  function typing(element: Element | null): boolean {
    return (
      element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement ||
      (element instanceof HTMLElement && element.isContentEditable)
    );
  }

  function onKeydown(event: KeyboardEvent) {
    if (keysClaimed()) return;
    if (event.key === 'Escape' && detailPane?.contains(event.target as Node)) {
      event.preventDefault();
      back();
      return;
    }
    const chord = mac ? event.metaKey : event.ctrlKey;
    if (!chord || event.shiftKey || event.altKey || event.key.toLowerCase() !== 'c') return;
    if (!row || typing(document.activeElement) || window.getSelection()?.toString()) return;
    const taken = copyOf(row);
    if (!taken) return;
    event.preventDefault();
    copy(taken.text, `${row.id}:${taken.which}`);
  }
</script>

<svelte:window on:keydown={onKeydown} />

<div class="history">
  {#if stacked}
    <div class="notices" role="region" aria-label="Notices" use:tabWhenScrolls>
      {#if shownHost !== null}
        <HostLine host={shownHost} inset />
      {/if}
      <slot name="alerts" />
      {#if waiting}
        <div class="notice">
          <p>An agent asked a question and is waiting for your answer.</p>
          {#if compositor}
            <p class="how">Press your compositor's Banshee binding to answer.</p>
          {:else if hotkey}
            <p class="how">{verb} <span class="mono">{hotkey}</span> to answer.</p>
          {/if}
        </div>
      {/if}
      {#if savingOff}
        <div class="notice">
          <p>Save history is off, so nothing new is kept.</p>
          <button class="btn btn-ghost" on:click={() => go('Saved history')}>
            Open Saved history
          </button>
        </div>
      {/if}
    </div>
  {/if}

  {#if rows.length > 0}
    <!-- A press only holds the row; a keyboard holds it through focusin. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="split" class:narrow={!wide} on:focusin={hold} on:pointerdown={hold}>
      <div class="pane list" class:hidden={reading} inert={reading}>
        {#if $listed.length > 0}
          <HistoryList
            bind:this={list}
            rows={$listed}
            selectedId={row?.id ?? null}
            {choose}
            {open}
            scrolled={hold}
            follow={$view.chosen === null}
            opensOnClick={!wide}
            query={$needle}
            now={$today}
          />
        {:else}
          <div class="none">
            <Absence label="No match" detail={`Nothing said so far contains “${$needle}”.`} />
          </div>
        {/if}
      </div>
      {#if wide || reading}
        <div class="pane" bind:this={detailPane}>
          {#if row}
            <HistoryDetail
              bind:this={detail}
              {row}
              now={$today}
              query={$needle}
              goListening={() => go('Listening')}
            />
          {/if}
        </div>
      {/if}
    </div>
  {:else if $table.loaded}
    <div class="empty">
      {#if !saving}
        <Absence
          label="Save history is off, so nothing new is kept."
          action="Open Saved history"
          act={() => go('Saved history')}
        />
      {:else if !downloading}
        <Absence
          label="Nothing here yet"
          detail={empty.detail}
          action={empty.action}
          act={() => empty.to && go(empty.to)}
        />
      {/if}
    </div>
  {/if}
</div>

<style>
  .history {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .notices {
    flex: none;
    max-height: 50vh;
    overflow-y: auto;
    padding-top: 18px;
    border-bottom: 1px solid var(--rule);
  }

  .notice {
    max-width: 520px;
    margin: 0 var(--gutter) 18px;
  }

  .notice p {
    margin: 0;
  }

  .notice .btn {
    margin-top: 12px;
  }

  .how {
    margin-top: 8px;
    font-size: 13px;
    color: var(--dim);
  }

  .split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(260px, 330px) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
  }

  .split.narrow {
    grid-template-columns: minmax(0, 1fr);
  }

  .pane {
    min-width: 0;
    min-height: 0;
  }

  .split:not(.narrow) .list {
    border-right: 1px solid var(--rule);
  }

  .narrow .pane {
    grid-area: 1 / 1;
  }

  /* Never `display: none`: the list at zero height mounts every row, and a
     hidden pane keeps its scroll offset. */
  .hidden {
    visibility: hidden;
  }

  .none {
    padding-top: 22px;
  }

  .empty {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-top: 22px;
  }
</style>
