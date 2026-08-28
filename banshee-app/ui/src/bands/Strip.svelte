<script lang="ts">
  import { daemon, stateWord } from '../lib/daemon';
  import { JOBS, OPENABLE, open, type Job } from '../lib/jobs';
  import Chevron from '../controls/Chevron.svelte';

  export let values: Partial<Record<Job, string>> = {};

  // A daemon that is not running knows none of these, so the rows carry no
  // value and open nothing.
  $: running = stateWord($daemon) !== 'Not running';
</script>

<nav aria-label="Settings" style="background: var(--strip); border-top: 1px solid var(--rule); padding: 8px 22px 12px; display: grid; grid-template-columns: 1fr 1fr; gap: 0 18px;">
  {#each JOBS as job, i (job)}
    {@const value = running ? (values[job] ?? '') : ''}
    {@const on = $open === job}
    {@const openable = OPENABLE.includes(job)}
    <!-- What is open can always be closed, whatever the daemon is doing. -->
    {@const silent = !openable || (value === '' && !on)}
    <button
      type="button"
      aria-expanded={openable ? on : undefined}
      disabled={silent}
      onclick={() => open.update((current) => (current === job ? null : job))}
      style="font: inherit; display: flex; justify-content: space-between; align-items: center; gap: 8px; min-height: 36px; padding: 0; background: transparent; border: 0; border-bottom: {i >= JOBS.length - 2 ? '0' : '1px solid var(--rule)'}; color: var(--ink); cursor: {silent ? 'default' : 'pointer'}; text-align: left;"
    >
      <span style="color: {on ? 'var(--ink)' : 'var(--dim)'}; font-weight: {on ? 600 : 400};">{job}</span>
      <span style="display: flex; align-items: center; gap: 4px; min-width: 0;">
        <span style="white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">{value}</span>
        {#if !silent}<Chevron up={on} />{/if}
      </span>
    </button>
  {/each}
</nav>
