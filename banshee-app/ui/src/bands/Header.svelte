<script lang="ts">
  import Mark from '../marks/Mark.svelte';
  import type { LampForm, Word } from '../lib/daemon';
  export let word: Word;
  export let form: LampForm;
</script>

<header class="band">
  <span class="mark"><Mark {form} size={34} /></span>
  <span class="caps state">{word}</span>
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

  .mark {
    display: flex;
    color: var(--ink);
  }
</style>
