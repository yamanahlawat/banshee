<script lang="ts">
  import { showCommands } from '../lib/settings';
  import Toggle from '../controls/Toggle.svelte';
  export let name: string;
</script>

<section aria-label={name} style="border-top: 1px solid var(--rule); padding: 12px 22px 14px; display: flex; flex-direction: column;">
  <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 4px;">
    <span class="caps" style="color: var(--dim);">{name}</span>
    <div style="display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--dim);">
      <span>Show commands</span>
      <Toggle on={$showCommands} label="Show commands" change={(next) => showCommands.set(next)} />
    </div>
  </div>
  <slot />
</section>
