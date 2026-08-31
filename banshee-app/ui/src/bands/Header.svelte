<script lang="ts">
  import Mark from '../marks/Mark.svelte';
  import type { LampForm, Word } from '../lib/daemon';
  export let word: Word;
  export let form: LampForm;
  export let waiting = false;
  export let restart: () => void;
  export let restarting = false;
</script>

<header class="band">
  <span class="mark"><Mark {form} size={34} /></span>
  <span class="caps state">{word}</span>
  {#if waiting}
    <button class="caps waiting btn-underline" on:click={restart} disabled={restarting}>
      {restarting ? 'Restarting' : 'Restart to apply'}
    </button>
  {/if}
</header>

<style>
  .band {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px var(--gutter);
    border-bottom: 1px solid var(--rule);
    flex: none;
  }

  .state {
    color: var(--accent);
  }

  .mark {
    display: flex;
    color: var(--ink);
  }

  /* The window can do this, so it is the control and not a note about one. */
  .waiting {
    margin-left: auto;
    color: var(--accent);
  }
</style>
