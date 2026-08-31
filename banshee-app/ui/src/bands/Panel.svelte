<script lang="ts">
  import { onMount } from 'svelte';

  // A job takes over the body rather than opening beside it, so the window
  // never has two things competing for the same attention, and so first run
  // has somewhere whole to land later.
  export let name: string;
  /// What is true of this job right now, read from the daemon. It is the
  /// heading, because a heading may not be smaller than the text it heads and
  /// the statement is the largest thing here.
  export let lead: string;
  export let close: () => void;

  let title: HTMLHeadingElement;

  // The body this replaced held the control that opened it, so the keyboard
  // has to be brought across or it is left on a node that is gone.
  onMount(() => title.focus());
</script>

<!-- The short name lives in the label, not above the statement: a small
     eyebrow over a heading is banned outright, and the statement already says
     which job you are in. -->
<section class="panel" aria-label={name}>
  <!-- The heading leads in the DOM because focus lands on it and the way out
       cannot sit behind every control in the panel. The float is visual only. -->
  <div class="head">
    <h2 class="title" tabindex="-1" bind:this={title}>{lead}</h2>
    <button class="close caps" on:click={close}>Done</button>
  </div>
  <div class="content">
    <slot />
  </div>
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .head {
    position: relative;
    padding: 0 var(--gutter) 26px;
  }

  .close {
    position: absolute;
    top: 0;
    right: var(--gutter);
  }

  /* The heading leads in the DOM so the keyboard reaches Done, but a float only
     wraps what follows it. This reserves the corner inside the statement's own
     flow, and the button sits over the space it keeps. */
  .title::before {
    content: '';
    float: right;
    width: 96px;
    height: 42px;
  }

  /* Focused only to carry the keyboard in, so it takes no ring: a reader who
     moved nothing would be told a heading is a control. */
  .title:focus {
    outline: none;
  }

  /* The lead treatment, in the agent cut: Banshee stating what is true of
     itself, in the same voice it uses for everything it says. */
  .title {
    margin: 0;
    max-width: 520px;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 28px;
    line-height: 1.3;
    letter-spacing: -0.03em;
  }

  .close {
    color: var(--ink);
    background: transparent;
    border: 1px solid var(--ink);
    border-radius: 0;
    padding: 6px 12px;
    cursor: pointer;
  }

  .close:hover {
    background: var(--ink);
    color: var(--ground);
  }

  .content {
    padding: 0 var(--gutter) 24px;
  }
</style>
