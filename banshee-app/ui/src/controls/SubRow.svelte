<script lang="ts">
  import { PENDING_SAYS } from '../lib/copy';

  export let name: string;
  export let pending = false;
  /// What the wait is for, where a restart is not it. A preset whose model is
  /// absent waits on a download, and a restart with nothing on disk to load
  /// changes nothing.
  export let says: string | null = null;
</script>

<div class="part">
  <span class="sub mono">{name}</span>
  <div class="control"><slot /></div>
  {#if pending}
    <p class="note pending">{says ?? PENDING_SAYS}</p>
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
