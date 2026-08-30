<script lang="ts">
  import { formatCount } from '../lib/history';
  import SaveSwitch from '../controls/SaveSwitch.svelte';

  export let total: number;
  export let saving: boolean;
  export let open: () => void;
</script>

<div class="ledger">
  <button class="state mono btn-underline" on:click={open}>
    {saving ? (total > 0 ? `${formatCount(total)} saved` : 'Nothing saved yet') : 'Not saving'}
    <span class="sr">— open what Banshee keeps</span>
  </button>

  <SaveSwitch {saving} />

  {#if saving && total > 1}
    <span class="hint mono">&#8984;F to find</span>
  {/if}
</div>

<style>
  .ledger {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 var(--gutter) 22px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--rule);
  }

  .state,
  .hint {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .state {
    color: var(--accent);
  }

  /* Lines up with the copy controls on the turns below. */
  .hint {
    margin-left: auto;
    color: var(--dim);
  }
</style>
