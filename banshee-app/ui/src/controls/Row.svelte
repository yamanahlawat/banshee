<script lang="ts">
  import { showCommands } from '../lib/settings';
  import Command from './Command.svelte';
  export let name: string;
  export let command: string | undefined = undefined;
  export let pending = false;
</script>
<div style="display: grid; grid-template-columns: 124px minmax(0, 1fr); gap: 4px 14px; align-items: center; padding: 9px 0; border-top: 1px solid var(--rule);">
  <span>{name}</span>
  <div style="display: flex; align-items: center; gap: 10px; min-width: 0; flex-wrap: wrap;">
    <slot />
    {#if pending}
      <span class="caps" style="color: var(--dim);">Pending</span>
    {/if}
  </div>
  {#if command && $showCommands}
    <span></span>
    <Command text={command} id={`command:${name}`} />
  {/if}
</div>
