<script lang="ts">
  import { RESTART_SAYS } from '../lib/copy';
  import { arrowStep } from '../lib/keys';
  // Four values, and the brief holds it to four. This band reports what
  // Banshee is set to. It is a status line that can be opened, not the
  // window's navigation, which is what went wrong with the strip it replaces.
  export let values: { id: string; label: string; value: string; pending?: boolean }[];
  export let open: (label: string, id: string) => void;
  export let active: string | null = null;

  let band: HTMLElement;

  // A toolbar is one tab stop and the arrows move within it. The foot is the
  // only route to every setting in the window, so it cannot sit behind every
  // copy control on the page.
  function onKeydown(event: KeyboardEvent) {
    const cells = [...band.querySelectorAll('button')];
    const at = cells.indexOf(event.currentTarget as HTMLButtonElement);
    const to = arrowStep(event.key, at, cells.length);
    if (to === null) return;
    event.preventDefault();
    stop = to;
    // Focus moves; nothing opens. A panel is a choice, not a side effect of
    // arriving somewhere.
    cells[to].focus();
  }

  // Returning by Tab lands where you left: the arrows move the stop, and
  // opening a panel moves it to that cell.
  let stop = 0;
  $: if (active) stop = Math.max(0, values.findIndex((row) => row.label === active));

  // A cell is a quarter of 480pt, so a device name is often cut. The whole of
  // it is in the DOM either way, and a screen reader reads it whole, so this
  // is for the eye alone: a tooltip on a value that fits would be noise, and a
  // description read back after the name it has already said.
  function clipped(node: HTMLElement, value: string) {
    let text = value;
    const mark = () => {
      if (node.scrollWidth > node.clientWidth) node.title = text;
      else node.removeAttribute('title');
    };
    mark();
    // Until Archivo lands the widths are the fallback's, so a value that fits
    // at first paint may not once the real face is measured.
    document.fonts?.ready?.then(mark);
    return {
      update(next: string) {
        text = next;
        mark();
      },
    };
  }
</script>

<footer class="band">
  <div class="cells" role="toolbar" aria-label="Jobs" bind:this={band}>
  {#each values as row, i (row.label)}
    <button
      id={row.id}
      class="cell"
      class:on={active === row.label}
      aria-pressed={active === row.label}
      tabindex={i === stop ? 0 : -1}
      on:keydown={onKeydown}
      on:click={() => {
        stop = i;
        open(row.label, row.id);
      }}
    >
      <span class="caps">{row.label}</span>
      <span class="mono value" class:pending={row.pending} use:clipped={row.value}>
        {row.value || '—'}
      </span>
      {#if row.pending}<span class="sr">{RESTART_SAYS}</span>{/if}
    </button>
    {/each}
  </div>
</footer>

<style>
  .band {
    padding: 12px var(--gutter) 14px;
    background: var(--foot);
    border-top: 1px solid var(--rule);
    flex: none;
  }

  /* The role sits here and not on the footer: a toolbar is not an allowed role
     on that element, and axe is right to say so. */
  .cells {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
  }

  .caps {
    color: var(--accent);
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
