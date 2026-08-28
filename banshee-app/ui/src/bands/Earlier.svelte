<script lang="ts">
  import type { HistoryRow } from '../lib/tauri';
  import { copy, copied } from '../lib/copy';
  import { formatTime } from '../lib/time';
  import { formatCount } from '../lib/history';
  import { open } from '../lib/jobs';

  export let rows: HistoryRow[];
  // Everything History holds that this band does not show.
  export let more: number;
  // `unread` is not `empty`: a daemon the window never reached says nothing
  // about whether anything was ever recorded.
  export let history: 'unread' | 'empty' | 'some' = 'unread';

  let expanded = new Set<string | number>();

  function toggleRow(id: string | number) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

</script>

<section aria-label="Earlier today" style="border-top: 1px solid var(--rule); padding: 12px 22px 8px;">
  <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; gap: 12px;">
    <span class="caps" style="color: var(--dim);">Earlier today</span>
  </div>
  {#if history === 'unread'}
    <p style="margin: 0; color: var(--dim);">History unread</p>
  {:else if history === 'empty'}
    <p style="margin: 0; color: var(--dim);">Nothing saved yet</p>
  {:else}
    {#if rows.length === 0}
      <p style="margin: 0 0 6px; color: var(--dim);">Nothing said today</p>
    {/if}
    <ol style="margin: 0; padding: 0; list-style: none;">
      {#each rows as row, i (row.id)}
        <li style="display: grid; grid-template-columns: 40px minmax(0, 1fr) 28px; gap: 10px; align-items: center; padding: 6px 0; border-top: {i ? '1px solid var(--rule)' : '0'};">
          <span class="mono" style="color: var(--dim);">{formatTime(row.timestamp)}</span>
          <button
            type="button"
            aria-label={`Show the whole dictation from ${formatTime(row.timestamp)}`}
            onclick={() => toggleRow(row.id)}
            style="font: inherit; text-align: left; padding: 0; border: 0; background: transparent; color: var(--ink); cursor: pointer; {expanded.has(row.id) ? 'white-space: normal;' : 'white-space: nowrap; overflow: hidden; text-overflow: ellipsis;'}"
          >{row.text}</button>
          {#if $copied === `history:${row.id}`}
            <span style="font-size: 12px; font-weight: 600; color: var(--ink); display: inline-flex; align-items: center; justify-content: center; min-height: 28px;">Copied</span>
          {:else}
            <button
              type="button"
              aria-label={`Copy the dictation from ${formatTime(row.timestamp)}`}
              title={`Copy the dictation from ${formatTime(row.timestamp)}`}
              onclick={() => copy(row.text, `history:${row.id}`)}
              style="width: 28px; height: 28px; border: 0; background: transparent; color: var(--dim); display: flex; align-items: center; justify-content: center; cursor: pointer; padding: 0; border-radius: 4px; flex-shrink: 0;"
            >
              <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
                <rect x="4.5" y="4.5" width="8" height="8" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.3" />
                <path d="M9.5 4.5 V3 a1.5 1.5 0 0 0 -1.5 -1.5 H3 A1.5 1.5 0 0 0 1.5 3 v5 a1.5 1.5 0 0 0 1.5 1.5 h1.5" fill="none" stroke="currentColor" stroke-width="1.3" />
              </svg>
            </button>
          {/if}
        </li>
      {/each}
      {#if more > 0}
        <li style="padding: 6px 0 2px 50px;">
          <button
            type="button"
            onclick={() => open.set('More settings')}
            style="font: inherit; font-size: 12.5px; color: var(--dim); background: transparent; border: 0; padding: 0; cursor: pointer;"
          >{formatCount(more)} more in History ›</button>
        </li>
      {/if}
    </ol>
  {/if}
</section>
