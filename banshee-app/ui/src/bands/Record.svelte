<script lang="ts">
  import { getContext, tick } from 'svelte';
  import { forget, formatCount, table } from '../lib/history';
  import { PANEL, type PanelFocus } from './panel';
  import { clearHistory } from '../lib/tauri';
  import { announce, report } from '../lib/copy';
  import SaveSwitch from '../controls/SaveSwitch.svelte';

  // Deleting is not undoable and not reversible from anywhere else, so it
  // asks once before it acts.
  let confirming = false;

  export let saving: boolean;
  $: total = $table.total;

  const panel = getContext<PanelFocus>(PANEL);

  let keep: HTMLButtonElement;
  let clearer: HTMLButtonElement;
  let deleting = false;

  // Each branch destroys the control that was just pressed. Without a move the
  // focus falls to the body, and a reader who cannot see the screen has to Tab
  // from the top of the document to reach the question they asked for.
  async function show(next: boolean, land: () => HTMLElement | undefined) {
    confirming = next;
    await tick();
    land()?.focus();
  }

  // `confirming` closes first: the round trip is not instant, and a second
  // press on a live Delete would run the whole thing again.
  async function clear() {
    // `disabled` is not enough: Svelte removes this button on the next tick, and
    // a press that landed before then still reaches the handler on the node.
    if (deleting) return;
    deleting = true;
    await show(false, () => clearer);
    try {
      await clearHistory();
      forget();
      // A delete that works takes Clear with it, so the panel says where next.
      panel?.refocus();
      announce('History cleared.');
    } catch {
      report('The record could not be cleared. Nothing was deleted.');
    }
    deleting = false;
  }
</script>

<div class="record">
  {#if confirming}
    <!-- On one line with the buttons this wraps at 480 and drops the
         destructive control alone at the gutter, which is the strongest
         reading position in the panel. -->
    <p class="warn">Delete all {formatCount(total)}? This cannot be undone.</p>
    <div class="actions">
      <button bind:this={keep} class="btn" on:click={() => show(false, () => clearer)}>
        Keep it
      </button>
      <button class="btn btn-ghost" disabled={deleting} on:click={clear}>Delete everything</button>
    </div>
  {:else}
    <div class="actions">
      <SaveSwitch {saving} />
      {#if total > 0}
        <button bind:this={clearer} class="btn btn-ghost" on:click={() => show(true, () => keep)}>
          Clear
        </button>
      {/if}
    </div>
  {/if}

  <p class="note">It lives in a file only you can read. Nothing is sent anywhere.</p>

  {#if !saving}
    <p class="note">
      Banshee is not keeping what you say. Dictation still works and still lands in the app you are
      using.
    </p>
  {/if}
</div>

<style>
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
  }

  .warn {
    max-width: 520px;
    margin: 0 0 12px;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    color: var(--accent);
  }

  .note {
    max-width: 520px;
    margin: 12px 0 0;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    line-height: 1.45;
  }
</style>
