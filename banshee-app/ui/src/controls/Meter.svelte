<script lang="ts">
  export let level: number;
  export let live: boolean;
  // Bar heights scale between a flat resting line and this waveform shape as level rises.
  const shape = [4, 6, 10, 14, 18, 12, 8, 14, 20, 16, 10, 6, 8, 12, 18, 22, 14, 8, 6, 10, 14, 10, 6, 4, 8, 12, 16, 10, 6, 4];
  $: clamped = Math.max(0, Math.min(100, level));
  $: fraction = clamped / 100;
  $: heights = shape.map((h) => 3 + (h - 3) * fraction);
  $: valuetext = clamped < 5 ? 'Quiet' : 'Speaking';
</script>
<div
  role="meter"
  aria-label="Input level"
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={clamped}
  aria-valuetext={valuetext}
  style="display: flex; align-items: flex-end; gap: 4px; height: 24px;"
>
  {#each heights as h}
    <div style="width: 6px; height: {h}px; background: {live ? 'var(--live)' : 'var(--dim)'}; border-radius: 1px;"></div>
  {/each}
</div>
