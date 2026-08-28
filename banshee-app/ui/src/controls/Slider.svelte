<script lang="ts">
  export let label: string;
  export let value: number;
  export let min: number;
  export let max: number;
  export let step: number;
  export let change: (next: number) => void;
</script>
<!-- A native range: the arrow keys and VoiceOver gestures come with it. -->
<input
  type="range"
  aria-label={label}
  {min}
  {max}
  {step}
  {value}
  onchange={(event) => change(Number(event.currentTarget.value))}
  style="flex: 1; min-width: 120px; height: 30px; margin: 0; accent-color: var(--ink); background: transparent;"
/>
