<script lang="ts">
  import { copy, copied } from '../lib/copy';
  export let text: string;
  export let id: string;
  $: done = $copied === id;
</script>
<div style="display: flex; align-items: flex-start; gap: 4px; min-width: 0;">
  <code class="mono" style="color: var(--dim); display: block; overflow-wrap: anywhere; line-height: 1.5; flex: 1;">{text}</code>
  {#if done}
    <span style="font-size: 12px; font-weight: 600; color: var(--ink); display: inline-flex; align-items: center; min-height: 28px; padding: 0;">Copied</span>
  {:else}
    <button
      type="button"
      aria-label="Copy command"
      title="Copy command"
      onclick={() => copy(text, id)}
      style="width: 28px; height: 28px; border: 0; background: transparent; color: var(--dim); display: flex; align-items: center; justify-content: center; cursor: pointer; padding: 0; border-radius: 4px; flex-shrink: 0;"
    >
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
        <rect x="4.5" y="4.5" width="8" height="8" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.3" />
        <path d="M9.5 4.5 V3 a1.5 1.5 0 0 0 -1.5 -1.5 H3 A1.5 1.5 0 0 0 1.5 3 v5 a1.5 1.5 0 0 0 1.5 1.5 h1.5" fill="none" stroke="currentColor" stroke-width="1.3" />
      </svg>
    </button>
  {/if}
</div>
