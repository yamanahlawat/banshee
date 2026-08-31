<script lang="ts">
  import { formatCount } from '../lib/history';
  import SaveSwitch from '../controls/SaveSwitch.svelte';

  export let total: number;
  export let saving: boolean;
  export let open: () => void;
  export let id: string;
</script>

<div class="ledger">
  <!-- The brief already calls this line the record's header. Saying so in the
       markup is what lets a screen reader jump to it. -->
  <h2>
    <button {id} class="state caps mono btn-underline" on:click={open}>
      {saving ? (total > 0 ? `${formatCount(total)} saved` : 'Nothing saved yet') : 'Not saving'}
      <span class="sr">— open what Banshee keeps</span>
    </button>
  </h2>

  <SaveSwitch {saving} />

  {#if saving && total > 1}
    <span class="hint caps mono">&#8984;F to find</span>
  {/if}
</div>

<style>
  .ledger {
    display: flex;
    align-items: center;
    gap: 16px;
    margin: 0 var(--gutter) 22px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--rule);
  }

  /* Flex, so the heading takes the button's height instead of adding a line
     box of its own and pushing the rule down. */
  h2 {
    display: flex;
    margin: 0;
    font: inherit;
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
