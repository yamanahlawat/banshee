<script lang="ts">
  export let name: string;
  export let note = '';
  export let pending = false;
</script>

<div class="field">
  <span class="caps">{name}</span>
  <div class="control"><slot /></div>
  {#if pending}
    <p class="note pending">Set. It takes effect when Banshee restarts.</p>
  {:else if note}
    <p class="note">{note}</p>
  {/if}
</div>

<style>
  .caps {
    color: var(--accent);
  }

  /* Separation by void, not by rule. */
  .field {
    margin-bottom: 26px;
  }

  .control {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
    min-width: 0;
  }

  .note {
    max-width: 520px;
    margin: 8px 0 0;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 13px;
    line-height: 1.45;
  }

  .pending {
    color: var(--accent);
  }
</style>
