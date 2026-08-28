<script lang="ts">
  export let options: { value: string; label: string }[];
  export let value: string;
  export let label: string;
  export let wide = true;
  export let change: (next: string) => void;
</script>
<!-- A native select: the platform draws the list, and the keyboard and
     VoiceOver behaviour come with it. -->
<label style="display: inline-flex; align-items: center; min-height: 30px; padding: 0 10px; border: 1.5px solid var(--ink); border-radius: 6px; background: var(--field); color: var(--ink); {wide ? 'flex: 1; min-width: 0;' : ''}">
  <span class="sr">{label}</span>
  <select
    value={value}
    onchange={(event) => change(event.currentTarget.value)}
    style="font: inherit; color: inherit; background: transparent; border: 0; padding: 0; margin: 0; width: 100%; min-width: 0; cursor: pointer; appearance: none; outline-offset: 4px;"
  >
    {#each options as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>
</label>
