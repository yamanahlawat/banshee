<script lang="ts">
  import { refusalId, refused } from '../lib/settings';

  export let setting: string;

  $: says = $refused.get(setting);
</script>

<!-- Not live: the window's announcement already says it, once. -->
{#if says !== undefined}
  <p class="note failed" id={refusalId(setting)}>{says}</p>
{/if}
