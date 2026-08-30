<script lang="ts">
  import { RESTART_SAYS } from '../lib/copy';
  // Four values, and the brief holds it to four. This band reports what
  // Banshee is set to. It is a status line that can be opened, not the
  // window's navigation, which is what went wrong with the strip it replaces.
  export let values: { label: string; value: string; pending?: boolean }[];
  export let open: (label: string) => void;
  export let active: string | null = null;
</script>

<footer class="band">
  {#each values as row (row.label)}
    <button
      class="cell"
      class:on={active === row.label}
      aria-pressed={active === row.label}
      on:click={() => open(row.label)}
    >
      <span class="caps">{row.label}</span>
      <span class="mono value" class:pending={row.pending}>{row.value || '—'}</span>
      {#if row.pending}<span class="sr">{RESTART_SAYS}</span>{/if}
    </button>
  {/each}
</footer>

<style>
  .band {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
    padding: 12px var(--gutter) 14px;
    background: var(--foot);
    border-top: 1px solid var(--rule);
    flex: none;
  }

  .cell {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 5px;
    min-width: 0;
    padding: 4px 0;
    background: transparent;
    border: 0;
    border-top: 2px solid transparent;
    border-radius: 0;
    cursor: pointer;
    text-align: left;
  }

  .cell.on {
    border-top-color: var(--accent);
  }

  .value {
    font-size: 11px;
    color: var(--ink);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cell:hover .value {
    color: var(--accent);
  }

  /* The dash is this world's form for a thing that is not there yet. The cell
     opens the panel that says it in words. */
  .value.pending {
    border-bottom: 1px dashed var(--accent);
  }
</style>
