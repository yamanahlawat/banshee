<script lang="ts">
  import { daemon } from '../lib/daemon';
  import { copy, copied } from '../lib/copy';
  import { formatTime } from '../lib/time';
  import Filled from '../controls/Filled.svelte';

  export let latest: { text: string; timestamp: string } | null;
  export let landing: string | null;
  export let agent: { who: string; text: string } | null;

  $: done = $copied === 'latest';
</script>

<section aria-label="Latest dictation" style="padding: 18px 22px 16px; display: flex; flex-direction: column; gap: 8px; flex: 1;">
  {#if agent}
    <div style="display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border-left: 2px solid var(--ink); margin-bottom: 14px;">
      <span class="caps" style="color: var(--dim);">{agent.who} asks</span>
      <p style="margin: 0; font-size: 15px; line-height: 1.45;">{agent.text}</p>
      {#if $daemon.live.speaking}
        <!-- The daemon reports no agent, so App passes null and this block
             never renders. banshee.stop_speaking has no bridge either, so
             the button carries no press. -->
        <div style="align-self: flex-start; margin-top: 6px;">
          <Filled label="Stop speaking" press={() => {}} />
        </div>
      {:else}
        <p style="margin: 6px 0 0; color: var(--dim);">Answer by speaking.</p>
      {/if}
    </div>
  {/if}
  {#if landing !== null}
    <div style="display: flex; align-items: center; justify-content: space-between;">
      <span class="caps" style="color: var(--dim);">Landing now</span>
    </div>
    <p style="margin: 0; font-size: 17px; line-height: 1.5; text-wrap: pretty;">{landing}<span aria-hidden="true" style="display: inline-block; width: 2px; height: 1em; background: var(--ink); vertical-align: text-bottom; margin-left: 2px;"></span></p>
  {:else if latest}
    <div style="display: flex; align-items: center; justify-content: space-between;">
      <span class="caps" style="color: var(--dim);">Latest · {formatTime(latest.timestamp)}</span>
      {#if done}
        <span style="font-size: 12px; font-weight: 600; color: var(--ink); display: inline-flex; align-items: center; min-height: 28px; padding: 0 10px;">Copied</span>
      {:else}
        <button
          type="button"
          onclick={() => latest && copy(latest.text, 'latest')}
          style="font: inherit; font-size: 12px; font-weight: 600; color: var(--ink); background: transparent; border: 1.5px solid var(--ink); border-radius: 6px; padding: 4px 10px; min-height: 28px; cursor: pointer; display: inline-flex; align-items: center; gap: 6px;"
        >
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
            <rect x="4.5" y="4.5" width="8" height="8" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.3" />
            <path d="M9.5 4.5 V3 a1.5 1.5 0 0 0 -1.5 -1.5 H3 A1.5 1.5 0 0 0 1.5 3 v5 a1.5 1.5 0 0 0 1.5 1.5 h1.5" fill="none" stroke="currentColor" stroke-width="1.3" />
          </svg>
          Copy
        </button>
      {/if}
    </div>
    <p style="margin: 0; font-size: 17px; line-height: 1.5; text-wrap: pretty;">{latest.text}</p>
  {/if}
</section>
