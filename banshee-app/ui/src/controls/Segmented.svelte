<script lang="ts">
  export let options: { value: string; label: string }[];
  export let active: string;
  export let label: string;
  export let change: (next: string) => void;
</script>
<div role="radiogroup" aria-label={label} style="display: inline-flex; border: 1.5px solid var(--ink); border-radius: 6px; overflow: hidden;">
  {#each options as option (option.value)}
    {@const on = option.value === active}
    <button
      type="button"
      role="radio"
      aria-checked={on}
      onclick={() => change(option.value)}
      style="font: inherit; min-height: 30px; padding: 0 12px; border: 0; background: {on ? 'var(--ink)' : 'transparent'}; color: {on ? 'var(--face)' : 'var(--ink)'}; font-weight: {on ? 600 : 400}; cursor: pointer;"
    >{option.label}</button>
  {/each}
</div>
