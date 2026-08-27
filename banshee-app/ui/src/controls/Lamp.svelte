<script lang="ts">
  import type { LampForm } from '../lib/daemon';
  export let form: LampForm;
  const shroud = 'M21 70 L24 46 C24 27 34 14 50 14 C66 14 76 27 76 46 L79 70 C79 80 72 88 64 86 C57 84 55 72 50 72 C45 72 43 84 36 86 C28 88 21 80 21 70 Z';
</script>
<svg width="18" height="18" viewBox="0 0 100 100" aria-hidden="true">
  {#if form === 'recording'}
    <path d={shroud} fill="var(--live)" stroke="var(--live)" stroke-width="9" stroke-linejoin="round" />
  {:else}
    <path d={shroud} fill="none" stroke="var(--ink)" stroke-width="9" stroke-linejoin="round" stroke-dasharray={form === 'notrunning' ? '22 14' : undefined} />
    {#if form === 'speaking'}
      <path d="M8 40 C4 48 4 56 8 64" fill="none" stroke="var(--ink)" stroke-width="8" stroke-linecap="round" />
      <path d="M92 40 C96 48 96 56 92 64" fill="none" stroke="var(--ink)" stroke-width="8" stroke-linecap="round" />
    {/if}
  {/if}
</svg>
