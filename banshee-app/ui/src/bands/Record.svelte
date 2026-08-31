<script lang="ts">
  import { forget, formatCount, table } from '../lib/history';
  import { clearHistory } from '../lib/tauri';
  import { announce, report } from '../lib/copy';
  import { daemon } from '../lib/daemon';
  import SaveSwitch from '../controls/SaveSwitch.svelte';

  // Deleting is not undoable and not reversible from anywhere else, so it
  // asks once before it acts.
  let confirming = false;

  export let saving: boolean;
  $: total = $table.total;

  async function clear() {
    confirming = false;
    try {
      await clearHistory();
      forget();
      announce('History cleared.');
    } catch {
      report('The record could not be cleared. Nothing was deleted.');
    }
  }
</script>

<div class="record">
  <div class="actions">
    {#if confirming}
      <!-- Keeping is the primary and reads first. The destructive control held
           both, and neither of them named how much was about to go. -->
      <span class="warn">Delete all {formatCount(total)}? This cannot be undone.</span>
      <button class="btn" on:click={() => (confirming = false)}>Keep it</button>
      <button class="btn btn-ghost" on:click={clear}>Delete everything</button>
    {:else}
      <SaveSwitch {saving} />
      {#if total > 0}
        <button class="btn btn-ghost" on:click={() => (confirming = true)}>Clear</button>
      {/if}
    {/if}
  </div>

  <p class="note">It lives in a file only you can read. Nothing is sent anywhere.</p>

  {#if !saving}
    <p class="note">
      Banshee is not keeping what you say. Dictation still works and still lands in the app you
      are using.
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
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 13px;
    color: var(--accent);
  }

  .note {
    max-width: 520px;
    margin: 12px 0 0;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 13px;
    line-height: 1.45;
  }
</style>
