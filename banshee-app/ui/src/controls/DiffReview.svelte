<script lang="ts">
  import type { PlannedChange } from '../lib/tauri';

  export let plan: PlannedChange[];
  export let apply: () => void;
  export let cancel: () => void;
  /// The apply is with the daemon, which cannot take it back.
  export let busy = false;
  /// Bound by the page, which lands focus on Apply when the review opens.
  export let applyButton: HTMLButtonElement | undefined = undefined;

  // The path above the box already names the file, so both header lines go.
  function body(diff: string) {
    const created = /^--- \/dev\/null\n/.test(diff);
    return {
      created,
      lines: diff
        .replace(/^--- .*\n\+\+\+ .*\n/, '')
        .replace(/\n$/, '')
        .split('\n'),
    };
  }
</script>

<div class="review">
  <span class="caps">What this changes</span>
  {#each plan as change (change.diff)}
    {@const view = body(change.diff)}
    {#if change.path}<p class="path mono">
        {change.path}{view.created ? ' (new file)' : ''}
      </p>{/if}
    <!-- A box that scrolls needs a tab stop, or a keyboard cannot scroll it. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <pre
      class="diff"
      role="region"
      tabindex="0"
      aria-label={change.path
        ? view.created
          ? `New file ${change.path}`
          : `Changes to ${change.path}`
        : 'Command to run'}>{#each view.lines as line, i (i)}<span class="line">{line}</span
        >{/each}</pre>
  {/each}
  <div class="actions">
    <button
      class="btn holds"
      bind:this={applyButton}
      aria-disabled={busy ? 'true' : undefined}
      on:click={apply}
    >
      <span>{busy ? 'Applying' : 'Apply'}</span>
      <span class="held" aria-hidden="true">Applying</span>
    </button>
    <button class="btn btn-ghost" aria-disabled={busy ? 'true' : undefined} on:click={cancel}>
      Cancel
    </button>
  </div>
</div>

<style>
  .caps {
    display: block;
    margin-top: 8px;
    color: var(--accent);
  }

  .path {
    margin: 12px 0 4px;
    font-size: 11px;
    color: var(--accent);
  }

  .diff {
    margin: 0;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.5;
    white-space: pre-wrap;
    /* A whole number of rows minus half a row, so the last one is always cut
       and the box shows that it scrolls. Half the window, less a row, keeps
       Apply above the foot. */
    max-height: calc(round(down, min(36em, 50vh - 1.5em), 1.5em) - 0.75em);
    overflow: auto;
    border-left: 1px solid var(--rule);
    padding-left: 12px;
    font-variant-ligatures: none;
  }

  .line {
    display: block;
    min-height: 1.5em;
    padding-left: 1.2em;
    text-indent: -1.2em;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 18px;
  }
</style>
