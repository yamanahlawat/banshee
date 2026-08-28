<script lang="ts">
  import Chevron from './Chevron.svelte';

  export let options: { value: string; label: string }[];
  export let value: string;
  export let label: string;
  export let wide = true;
  export let change: (next: string) => void;

  // A select whose value names no option renders blank, which is how an
  // unplugged device reads as a missing control rather than a missing device.
  $: shown = options.some((o) => o.value === value)
    ? options
    : [{ value, label: value }, ...options];
</script>
<!-- A native select: the platform draws the list, and the keyboard and
     VoiceOver behaviour come with it. -->
<label style="display: inline-flex; align-items: center; gap: 8px; min-height: 30px; padding: 0 10px; border: 1.5px solid var(--ink); border-radius: 6px; background: var(--field); color: var(--ink); {wide ? 'flex: 1; min-width: 0;' : ''}">
  <span class="sr">{label}</span>
  <select
    value={value}
    onchange={(event) => change(event.currentTarget.value)}
    style="font: inherit; color: inherit; background: transparent; border: 0; padding: 0; margin: 0; width: 100%; min-width: 0; cursor: pointer; appearance: none; outline-offset: 4px;"
  >
    {#each shown as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>
  <Chevron />
</label>
