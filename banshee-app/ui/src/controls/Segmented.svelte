<script lang="ts">
  // A component and not a CSS class: without the group role and label a screen
  // reader hears N separate toggles with no idea they are one choice.
  import { arrowStep } from '../lib/keys';

  export let label: string;
  export let value: string;
  export let options: { value: string; label: string }[];
  export let change: (next: string) => void;

  let group: HTMLDivElement;

  // A radiogroup is one tab stop and the arrows move inside it. Saying it in
  // the role and then not doing it leaves a screen reader announcing a control
  // that will not answer.
  // A daemon value these options do not hold leaves no match, and every cell at
  // -1 puts the group out of the keyboard's reach.
  // A daemon round trip is not instant, so the handlers write this directly and
  // the next arrow steps from where the last one left. The statement re-runs
  // only when `value` or `options` moves, so the daemon's answer takes it back
  // whatever it says. `Math.max`, because a value these options do not hold
  // leaves -1, and that puts the group out of the keyboard's reach.
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
