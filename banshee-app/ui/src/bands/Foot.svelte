<script lang="ts">
  import { arrowStep } from '../lib/keys';
  /// `label` routes and `title` is read: the cell that opens the Microphone job
  /// says what it reports, which is where listening happens. `pending` is the
  /// sentence a screen reader hears when the value is not the one coming.
  export let values: {
    id: string;
    label: string;
    title?: string;
    value: string;
    pending?: string;
  }[];
  export let open: (label: string, id: string) => void;
  export let active: string | null = null;

  let band: HTMLElement;

  // A toolbar is one tab stop and the arrows move within it. The foot is the
  // only route to every setting in the window, so it cannot sit behind every
  // copy control on the page.
  const cells = () => [...band.querySelectorAll('button')];

  function onKeydown(event: KeyboardEvent) {
    const all = cells();
    const at = all.indexOf(event.currentTarget as HTMLButtonElement);
    const to = arrowStep(event.key, at, all.length);
    if (to === null) return;
    event.preventDefault();
    stop = to;
    // Focus moves; nothing opens. A panel is a choice, not a side effect of
    // arriving somewhere.
    all[to].focus();
  }

  // Returning by Tab lands where you left: the arrows move the stop, and
  // opening a panel moves it to that cell.
  let stop = 0;

  /// Lands on the cell the stop names, or a reader entering from outside
  /// arrives on one that Tab will not leave from.
  export function enter() {
    cells()[stop]?.focus();
  }
  $: if (active)
    stop = Math.max(
      0,
      values.findIndex((row) => row.label === active),
    );

  // For the eye alone: a screen reader reads the whole value, so a tooltip only on a clipped one.
  function clipped(node: HTMLElement, value: string) {
    let text = value;
    let frame = 0;
    let alive = true;
    // Measured after the frame that paints the value, or the width read is the
    // one the previous value had.
    const mark = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!alive) return;
        if (node.scrollWidth > node.clientWidth) node.title = text;
        else node.removeAttribute('title');
      });
    };
    mark();
    // Until Archivo lands the widths are the fallback's, so a value that fits
    // at first paint may not once the real face is measured.
    document.fonts?.ready?.then(() => {
      if (alive) mark();
    });
    return {
      update(next: string) {
        text = next;
        mark();
      },
      destroy() {
        alive = false;
        cancelAnimationFrame(frame);
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
        <span class="caps" use:clipped={row.title ?? row.label}>{row.title ?? row.label}</span>
        <span class="mono value" class:pending={row.pending} use:clipped={row.value}>
          {row.value || '—'}
        </span>
        {#if row.pending}<span class="sr">{row.pending}</span>{/if}
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

  /* The label clips the way the value does: at 140% zoom and above the four
     labels collide without it, and the tooltip keeps the full word. */
  .caps {
    color: var(--accent);
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  /* One mark at a time: the bar above says this cell is open, so the rule that
     offered the panel has nothing left to say. A value the daemon has not taken
     keeps its dashed mark, which is about the value and not about the cell. */
  .cell.on .value:not(.pending) {
    border-bottom-color: transparent;
  }

  /* The rule is the affordance: these four cells are the only route to every
     job in the window, and the accent and the top border both wait on an
     interaction. It is the treatment the ledger's controls use one band up. */
  .value {
    font-size: 11px;
    color: var(--ink);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border-bottom: 1px solid currentcolor;
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
