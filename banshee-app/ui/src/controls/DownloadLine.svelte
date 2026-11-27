<script lang="ts">
  import { daemon, downloadLine } from '../lib/daemon';
  import { samples } from '../lib/downloads';
  import { paceLine, paceOf } from '../lib/pace';

  $: download = $daemon.download;
  $: pace = paceLine(paceOf($samples));
</script>

{#if download}
  <p class="progress mono">
    {downloadLine(download)}
    {#if pace && download.state !== 'failed'}· <span class="pace">{pace}</span>{/if}
  </p>
{/if}

<style>
  .progress {
    margin: 10px 0 0;
    font-size: 11px;
    color: var(--accent);
  }

  .pace {
    display: inline-block;
  }
</style>
