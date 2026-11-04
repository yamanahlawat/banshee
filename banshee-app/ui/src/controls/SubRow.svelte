<script lang="ts">
  import { waiting } from '../lib/daemon';
  import { waitingSays } from '../lib/copy';

  export let name: string;
  // The key this part writes, when it writes one.
  export let setting: string | undefined = undefined;
  /// A wait the daemon does not report, said in place of the key's own line. A
  /// chosen model that is absent waits on a download.
  export let says: string | null = null;

  $: line = says ?? (setting !== undefined && $waiting.has(setting) ? waitingSays(setting) : null);
</script>

<div class="part">
  <span class="sub mono">{name}</span>
  <div class="control"><slot /></div>
  {#if line !== null}
    <p class="note pending">{line}</p>
  {/if}
  <!-- After the sentence that explains it, the way a drawn box orders its own
       head, consequence and fix. -->
  <div class="act"><slot name="action" /></div>
</div>

<style>
  .part {
    display: grid;
    grid-template-columns: 56px 1fr;
    column-gap: 12px;
    align-items: baseline;
  }

  /* Subordinate to the one accent caps name the group carries, so the mono role
     without the caps register, and the measured dim rather than an opacity. */
  .sub {
    font-size: 11px;
    color: var(--dim);
  }

  /* Under the control it is about, not under the name beside it. */
  .note {
    grid-column: 2;
    margin: 8px 0 0;
  }

  .act {
    display: contents;
  }
</style>
