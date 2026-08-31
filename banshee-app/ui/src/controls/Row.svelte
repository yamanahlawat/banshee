<script lang="ts">
  // One property of one job. The name is the specimen label, the value is the
  // specimen: the foot cell's own grammar, at panel scale.
  export let name: string;
  export let note = '';
  export let pending = false;
  /// A control that cannot sit on one line takes the width and drops beneath
  /// its label. Most properties are a line; some specimens need a block.
  export let block = false;
</script>

<div class="row" class:block>
  <span class="name caps">{name}</span>
  <div class="value">
    <!-- A row, because a control and its readout sit beside each other: the
         slider and its number are one reading, not two. -->
    <div class="control"><slot /></div>
    {#if pending}
      <p class="note pending">Set. It takes effect when Banshee restarts.</p>
    {:else if note}
      <p class="note">{note}</p>
    {/if}
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

  /* Centred, not stretched: Segmented sizes itself to its options, and a
     container that stretches it grows the last cell into space no option
     fills. */
  .control {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .note {
    max-width: 520px;
    margin: 8px 0 0;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    line-height: 1.45;
  }

  .pending {
    color: var(--accent);
  }
</style>
