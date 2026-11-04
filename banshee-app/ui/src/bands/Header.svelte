<script lang="ts">
  import Mark from '../marks/Mark.svelte';
  import type { LampForm, Word } from '../lib/daemon';
  export let word: Word;
  export let form: LampForm;
  /// What failed, in two words. A server's message never reaches this band:
  /// it would grow it to any width.
  export let failure: string | null = null;
  export let showFailure: () => void = () => {};
  export let failureId = '';
</script>

<header class="band">
  <span class="mark"><Mark {form} /></span>
  <span class="caps state">{word}</span>
  {#if failure}
    <button id={failureId} class="caps failed btn-underline" on:click={showFailure}>
      {failure}
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

  .failed {
    margin-left: auto;
    color: var(--accent);
    font-variation-settings:
      'wght' 700,
      'wdth' 100;
  }
</style>
