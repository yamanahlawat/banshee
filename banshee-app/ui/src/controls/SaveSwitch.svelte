<script lang="ts">
  import { waiting } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { WAITING_SAYS } from '../lib/copy';
  import { rowDescription } from '../lib/ids';

  export let saving: boolean;
  export let describedBy: string | undefined = undefined;
  const fromRow = rowDescription();
  $: described = describedBy ?? $fromRow;

  // The key is a constant of this control, so the one fact about it is read
  // here rather than computed by each band that places it.
  $: pending = $waiting.has('daemon.save_history');
</script>

<button
  class="btn btn-ghost"
  class:pending
  aria-describedby={described}
  on:click={() => write('daemon.save_history', !saving)}
>
  {saving ? 'Stop saving' : 'Start saving'}
  {#if pending}<span class="sr">{WAITING_SAYS}</span>{/if}
</button>

<style>
  /* The dashed accent line this world draws around a value the daemon has not
     taken. */
  .pending {
    border-style: dashed;
    border-color: var(--accent);
  }
</style>
