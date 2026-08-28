<script lang="ts">
  import { onMount } from 'svelte';
  import { daemon } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { clearHistory, history, type HistoryRow } from '../lib/tauri';
  import { formatCount, newestFirst, today } from '../lib/history';
  import { copy, copied } from '../lib/copy';
  import { formatTime } from '../lib/time';
  import Job from '../bands/Job.svelte';
  import Row from '../controls/Row.svelte';
  import More from '../controls/More.svelte';
  import Toggle from '../controls/Toggle.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import Action from '../controls/Action.svelte';
  import Filled from '../controls/Filled.svelte';

  const LEVELS = [
    { value: 'error', label: 'Quiet' },
    { value: 'info', label: 'Normal' },
    { value: 'debug', label: 'Everything' },
  ];

  let rows: HistoryRow[] = [];
  let loaded = false;
  let query = '';
  let confirming = false;
  let expanded = new Set<string | number>();

  onMount(async () => {
    try {
      rows = newestFirst(await history());
    } catch {
      rows = [];
    } finally {
      loaded = true;
    }
  });

  function toggleRow(id: string | number) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  async function clearAll() {
    await clearHistory();
    rows = [];
    confirming = false;
  }

  $: needle = query.trim().toLowerCase();
  $: todays = today(rows, new Date());
  // Unsearched, the list is today's dictations only, matching the caption
  // below it. A search reaches the whole table, since an older dictation is
  // still something the search field promises to find.
  $: filtered = needle === '' ? todays : rows.filter((row) => row.text.toLowerCase().includes(needle));
  $: olderCount = rows.length - todays.length;

  $: config = ($daemon.status?.config ?? {}) as Record<string, Record<string, unknown>>;
  $: daemonConfig = (config.daemon ?? {}) as Record<string, unknown>;
  $: saveHistory = daemonConfig.save_history !== false;
  $: alwaysOn = daemonConfig.always_on === true;
  $: level = String((config.logging ?? {}).level ?? 'info');
</script>

<section aria-label="History" style="padding: 14px 22px 8px; display: flex; flex-direction: column; gap: 10px; flex: 1;">
  {#if loaded && rows.length === 0}
    <p style="margin: 0; color: var(--dim);">Nothing saved yet</p>
  {:else}
    <div style="display: flex; align-items: center; gap: 10px;">
      <div style="flex: 1; display: flex; align-items: center; gap: 8px; min-height: 32px; padding: 0 10px; border: 1.5px solid var(--ink); border-radius: 6px; background: var(--field);">
        <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
          <circle cx="6" cy="6" r="4.5" fill="none" stroke="var(--ink)" stroke-width="1.5" />
          <path d="M9.5 9.5 L13 13" stroke="var(--ink)" stroke-width="1.5" stroke-linecap="round" />
        </svg>
        <input
          type="search"
          bind:value={query}
          placeholder={`Search ${formatCount(rows.length)} dictations`}
          aria-label="Search history"
          style="flex: 1; border: 0; background: transparent; font: inherit; color: var(--ink);"
        />
      </div>
      <Action label="Clear all" press={() => { confirming = true; }} />
    </div>

    {#if confirming}
      <div role="alertdialog" aria-label="Clear history" style="display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 12px; border-left: 2px solid var(--ink); background: var(--strip);">
        <span>Clear all {formatCount(rows.length)} dictations? This cannot be undone.</span>
        <div style="display: flex; gap: 8px;">
          <Filled label="Cancel" press={() => { confirming = false; }} />
          <Action label={`Clear ${formatCount(rows.length)}`} press={clearAll} />
        </div>
      </div>
    {/if}

    <ol style="margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column;">
      {#each filtered as row, i (row.id)}
        {@const open = expanded.has(row.id)}
        <li style="display: grid; grid-template-columns: 40px minmax(0, 1fr) 56px; gap: 10px; padding: 9px 0; border-top: {i ? '1px solid var(--rule)' : '0'}; align-items: start;">
          <span class="mono" style="color: var(--dim); padding-top: 3px;">{formatTime(row.timestamp)}</span>
          <button
            type="button"
            aria-expanded={open}
            aria-label={open ? 'Collapse' : `Show the whole dictation from ${formatTime(row.timestamp)}`}
            onclick={() => toggleRow(row.id)}
            style="font: inherit; text-align: left; padding: 0; border: 0; background: transparent; color: var(--ink); cursor: pointer; {open ? 'text-wrap: pretty;' : 'white-space: nowrap; overflow: hidden; text-overflow: ellipsis;'}"
          >{row.text}</button>
          <span style="display: flex; justify-content: flex-end;">
            {#if $copied === `history:${row.id}`}
              <span role="status" style="font-size: 12px; font-weight: 600; color: var(--ink); display: inline-flex; align-items: center; min-height: 28px;">Copied</span>
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
          </span>
        </li>
      {/each}
    </ol>
    {#if needle === ''}
      <p class="caps" style="margin: 0; color: var(--dim);">Today · {formatCount(olderCount)} older</p>
    {:else}
      <p class="caps" style="margin: 0; color: var(--dim);">{formatCount(filtered.length)} {filtered.length === 1 ? 'match' : 'matches'}</p>
    {/if}
    <p style="margin: 0; color: var(--dim); font-size: 12.5px;">Click a line to see all of it. Saved on this Mac only.</p>
  {/if}
</section>

<Job name="History settings">
  <Row name="Save what I say" command={`banshee config set daemon.save_history ${saveHistory}`} pending={$daemon.pending.has('daemon.save_history')}>
    <Toggle on={saveHistory} label="Save history" change={(next) => write('daemon.save_history', next)} />
    <span>{saveHistory ? 'On' : 'Off'}</span>
  </Row>

  <More />

  <Row name="Keep listening" command={`banshee config set daemon.always_on ${alwaysOn}`} pending={$daemon.pending.has('daemon.always_on')}>
    <Toggle on={alwaysOn} label="Always on" change={(next) => write('daemon.always_on', next)} />
    <span>{alwaysOn ? 'On' : 'Off. Banshee listens only while the key is held.'}</span>
  </Row>

  <Row name="Log detail" command={`banshee config set logging.level ${level}`} pending={$daemon.pending.has('logging.level')}>
    <Segmented label="Log detail" active={level} options={LEVELS} change={(next) => write('logging.level', next)} />
  </Row>
</Job>
