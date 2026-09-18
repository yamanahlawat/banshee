<script lang="ts">
  export let label: string;
  export let detail = '';
  export let action = '';
  export let act: (() => void) | null = null;
  export let busy = false;
  export let id = '';
  // Indented to the turn text column only where the box stands in for a turn.
  // Above the record it stands among bands and takes the gutter they share.
  export let inRecord = false;
</script>

<div class="absence" class:in-record={inRecord}>
  <h2 class="label">{label}</h2>
  {#if detail}<p class="detail">{detail}</p>{/if}
  {#if action && act}
    <button {id} class="btn" on:click={act} disabled={busy}>{action}</button>
  {/if}
</div>

<style>
  .absence {
    margin: 4px var(--gutter) 18px;
    padding: 14px 16px;
    border: 1px dashed var(--accent);
  }

  /* 52px time gutter and a 12px gap: the turn grid, from the other side. */
  .in-record {
    margin-left: calc(var(--gutter) + 64px);
  }

  .label {
    margin: 0;
    font-variation-settings:
      'wght' 750,
      'wdth' 106;
    font-size: 17px;
    line-height: 1.3;
    letter-spacing: -0.015em;
  }

  .detail {
    max-width: 520px;
    margin: 8px 0 0;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
    color: var(--ink);
  }

  .btn {
    margin-top: 14px;
  }
</style>
