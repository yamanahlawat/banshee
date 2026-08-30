<script lang="ts">
  // The platform draws its own bezel and stepper on a select unless the
  // appearance is reset, and `color-scheme` decides which bezel.
  export let label: string;
  export let value: string;
  export let change: (next: string) => void;
  export let disabled = false;

</script>

<span class="picker">
  <!-- svelte-ignore a11y_no_onchange -->
  <select
    aria-label={label}
    {value}
    {disabled}
    on:change={(e) => change(e.currentTarget.value)}
  >
    <slot />
  </select>
  <svg width="11" height="7" viewBox="0 0 11 7" aria-hidden="true" focusable="false">
    <path
      d="M1 1.5 L5.5 5.5 L10 1.5"
      fill="none"
      stroke="currentColor"
      stroke-width="1.4"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
</span>

<style>
  select:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .picker {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    color: var(--ink);
    border-bottom: 1px solid var(--ink);
  }

  select {
    appearance: none;
    -webkit-appearance: none;
    flex: 1;
    min-width: 0;
    font-variation-settings: 'wght' 500, 'wdth' 100;
    font-size: 15px;
    color: var(--ink);
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 6px 22px 6px 0;
    margin: 0;
  }

  svg {
    position: absolute;
    right: 2px;
    pointer-events: none;
  }
</style>
