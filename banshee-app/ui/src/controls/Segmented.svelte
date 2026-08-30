<script lang="ts">
  // A component and not a CSS class: without the group role and label a screen
  // reader hears N separate toggles with no idea they are one choice.
  export let label: string;
  export let value: string;
  export let options: { value: string; label: string }[];
  export let change: (next: string) => void;
</script>

<div class="seg" role="radiogroup" aria-label={label}>
  {#each options as option (option.value)}
    <button
      type="button"
      role="radio"
      aria-checked={option.value === value}
      on:click={() => change(option.value)}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  /* Scoped, so the rule that draws the chosen segment sits beside the attribute
     that marks it. Split across two files these drifted, and no segment
     inverted. */
  .seg {
    /* Inline, so the box is the width of its options. Stretched to a parent it
       shows a fourth cell that no option fills. */
    display: inline-flex;
    border: 1px solid var(--ink);
  }

  .seg button {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--ink);
    background: transparent;
    border: 0;
    border-right: 1px solid var(--ink);
    padding: 8px 12px;
    cursor: pointer;
  }

  .seg button:last-child {
    border-right: 0;
  }

  .seg button[aria-checked='true'] {
    background: var(--ink);
    color: var(--ground);
  }
</style>
