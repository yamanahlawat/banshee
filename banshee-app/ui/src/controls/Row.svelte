<script lang="ts">
  import { PENDING_SAYS } from '../lib/copy';

  export let name: string;
  export let note = '';
  /// Passed when a control has to point at the note with `aria-describedby`.
  export let noteId: string | undefined = undefined;
  export let pending = false;
  /// A control that cannot sit on one line takes the width and drops beneath its label.
  export let block = false;
</script>

<div class="row" class:block>
  <span class="name caps">{name}</span>
  <div class="value">
    <div class="control"><slot /></div>
    <!-- Both, and the note first: a pending line that replaces the note drops
         what the control means at the moment the reader changed it. -->
    {#if note}
      <p class="note" id={noteId}>{note}</p>
    {/if}
    {#if pending}
      <p class="note pending">{PENDING_SAYS}</p>
    {/if}
    <slot name="under" />
  </div>
</div>

<style>
  /* Separation by void, never a rule between properties. */
  .row {
    display: grid;
    grid-template-columns: 112px 1fr;
    column-gap: 12px;
    align-items: baseline;
    margin-bottom: 22px;
  }

  .block {
    grid-template-columns: 1fr;
    row-gap: 8px;
  }

  .name {
    color: var(--accent);
  }

  .value {
    min-width: 0;
  }

  .note {
    margin: 8px 0 0;
  }
</style>
