<script lang="ts">
  // A component and not a CSS class: without the group role and label a screen
  // reader hears N separate toggles with no idea they are one choice.
  import { arrowStep } from '../lib/keys';

  export let label: string;
  export let value: string;
  export let options: { value: string; label: string }[];
  export let change: (next: string) => void;

  let group: HTMLDivElement;

  // One tab stop, arrows inside it. The handlers write stop directly because the daemon round trip
  // is not instant; the statement re-runs only when value or options moves. Math.max: a value the
  // options do not hold would leave -1 and take the group out of keyboard reach, so the first stop
  // stands in.
  let stop = 0;
  $: stop = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );

  function pick(to: number) {
    stop = to;
    change(options[to].value);
  }

  function onKeydown(event: KeyboardEvent) {
    const to = arrowStep(event.key, stop, options.length);
    if (to === null) return;
    event.preventDefault();
    pick(to);
    // The write is the daemon's round trip, and the focus is not: it moves now
    // or the next Tab leaves from a cell nobody is on.
    (group.children[to] as HTMLElement).focus();
  }
</script>

<div class="seg" role="radiogroup" aria-label={label} bind:this={group}>
  {#each options as option, i (option.value)}
    <button
      class="caps"
      type="button"
      role="radio"
      aria-checked={option.value === value}
      tabindex={i === stop ? 0 : -1}
      on:click={() => pick(i)}
      on:keydown={onKeydown}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  /* Scoped, so the rule that draws the chosen segment sits beside the attribute that marks it. */
  .seg {
    /* Inline, so the box is the width of its options. Stretched to a parent it
       shows a fourth cell that no option fills. */
    display: inline-flex;
    border: 1px solid var(--ink);
  }

  .seg button {
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
