<script lang="ts">
  export let on: boolean;
  export let label: string;
  export let change: (next: boolean) => void;
</script>
<button
  type="button"
  role="switch"
  aria-checked={on}
  aria-label={label}
  onclick={() => change(!on)}
  style="width: 38px; height: 22px; border-radius: 11px; border: 1.5px solid var(--ink); background: {on ? 'var(--ink)' : 'transparent'}; position: relative; cursor: pointer; padding: 0; flex-shrink: 0;"
>
  <span style="position: absolute; top: 2px; {on ? 'right: 2px;' : 'left: 2px;'} width: 16px; height: 16px; border-radius: 50%; background: {on ? 'var(--face)' : 'var(--ink)'};"></span>
</button>
