<script lang="ts">
  import { onDestroy } from 'svelte';
  import { blocked, BUSY_NOTE, forgetBlocked, ONE_AT_A_TIME } from '../lib/downloads';

  /// The key the button claims the run with.
  export let button: string;

  onDestroy(() => forgetBlocked(button));
</script>

{#if $blocked === button}
  <p class="note busy" id={BUSY_NOTE}>{ONE_AT_A_TIME}</p>
{/if}

<style>
  .busy {
    margin: 8px 0 0;
  }
</style>
