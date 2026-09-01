<script lang="ts">
  import { waitsOnARestart } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { RESTART_SAYS } from '../lib/copy';

  export let saving: boolean;

  // The key is a constant of this control, so the one fact about it is read
  // here rather than computed by each band that places it.
  $: pending = $waitsOnARestart.has('daemon.save_history');
</script>

<button
  class="caps btn-underline"
  class:pending
  on:click={() => write('daemon.save_history', !saving)}
>
  {saving ? 'Stop saving' : 'Start saving'}
  {#if pending}<span class="sr">{RESTART_SAYS}</span>{/if}
</button>

<style>
  /* A resting rule, so it reads as a control rather than the second half of
     the count beside it. Without one, "8 saved  Stop saving" is one phrase. */
  button {
    color: var(--ink);
    border-bottom-color: currentcolor;
  }

  /* The dash this world uses for a value the daemon has not taken. */
  button.pending {
    border-bottom: 1px dashed var(--accent);
  }
</style>
