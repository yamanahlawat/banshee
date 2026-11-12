<script lang="ts">
  export let label: string;
  export let detail = '';
  export let action = '';
  export let act: (() => void) | null = null;
  export let busy = false;
  export let id = '';
</script>

<div class="absence">
  <h2 class="box-head">{label}</h2>
  {#if detail}<p class="detail">{detail}</p>{/if}
  {#if action && act}
    <button {id} class="btn" on:click={act} disabled={busy}>{action}</button>
  {/if}
</div>

<style>
  .absence {
    max-width: var(--measure);
    margin: 4px var(--gutter) 18px;
    padding: 14px 16px;
    border: 1px dashed var(--accent);
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
