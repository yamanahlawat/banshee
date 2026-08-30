<script lang="ts">
  import { formatCount } from '../lib/history';
  import { write } from '../lib/settings';

  export let total: number;
  export let saving: boolean;
  export let open: () => void;
</script>

<div class="ledger">
  <button class="state mono" on:click={open}>
    {saving ? (total > 0 ? `${formatCount(total)} saved` : 'Nothing saved yet') : 'Not saving'}
    <span class="sr">— open what Banshee keeps</span>
  </button>

  <button class="switch mono" on:click={() => write('daemon.save_history', !saving)}>
    {saving ? 'Stop saving' : 'Start saving'}
  </button>

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
  .switch,
  .hint {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .state {
    color: var(--accent);
    background: transparent;
    border: 0;
    border-bottom: 1px solid transparent;
    padding: 0;
    cursor: pointer;
    flex: none;
  }

  .state:hover {
    border-bottom-color: var(--accent);
  }

  .switch {
    color: var(--ink);
    background: transparent;
    border: 0;
    border-bottom: 1px solid transparent;
    border-radius: 0;
    padding: 0 0 1px;
    cursor: pointer;
    flex: none;
  }

  .switch:hover {
    border-bottom-color: var(--ink);
  }

  /* Lines up with the copy controls on the turns below. */
  .hint {
    margin-left: auto;
    color: var(--dim);
  }
</style>
