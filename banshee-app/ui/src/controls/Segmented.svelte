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
