<script lang="ts">
  import { copied, copy, failureSays } from '../lib/copy';

  export let id: string;
  export let label: string;
  export let said: string;

  /// The panel keeps one `id` for its failure row, so a second failure arriving
  /// inside the held confirmation would leave Copied over text nobody copied.
  $: copyId = `${id}:${said}`;
</script>

<div class="failure">
  <div {id} tabindex="-1">
    <p class="note failed">{label}</p>
    <p class="note said">{said}</p>
  </div>
  <button class="caps btn-underline" on:click={() => copy(failureSays(label, said), copyId)}>
    {$copied === copyId ? 'Copied' : 'Copy'}
    <span class="sr">the failure</span>
  </button>
</div>

<style>
  .failure {
    display: flex;
    flex-direction: column;
    align-items: start;
    gap: 6px;
  }

  /* Banshee's line above carries the weight. */
  .said {
    margin: 0;
    color: var(--dim);
  }
</style>
