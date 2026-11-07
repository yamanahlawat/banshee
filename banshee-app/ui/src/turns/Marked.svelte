<script lang="ts">
  import { marked } from '../lib/history';

  export let text: string;
  export let needle: string;
</script>

<!-- No whitespace between runs: it would draw as a space inside the words. -->
{#each marked(text, needle) as part, i (i)}{#if part.hit}<mark>{part.text}</mark
    >{:else}{part.text}{/if}{/each}

<style>
  mark {
    background: var(--accent);
    color: var(--ground);
  }
</style>
