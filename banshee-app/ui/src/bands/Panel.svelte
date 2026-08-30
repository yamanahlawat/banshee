<script lang="ts">
  import { onMount } from 'svelte';

  // A job takes over the body rather than opening beside it, so the window
  // never has two things competing for the same attention, and so first run
  // has somewhere whole to land later.
  export let name: string;
  export let close: () => void;

  let title: HTMLHeadingElement;

  // The body this replaced held the control that opened it, so the keyboard
  // has to be brought across or it is left on a node that is gone.
  onMount(() => title.focus());
</script>

<section class="panel" aria-label={name}>
  <div class="head">
    <!-- h2, not h1: a panel replaces the record rather than sitting inside it,
         and the window's name is the title bar's. -->
    <h2 class="title" tabindex="-1" bind:this={title}>{name}</h2>
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
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 var(--gutter) 20px;
  }

  /* Focused only to carry the keyboard in, so it takes no ring: a reader who
     moved nothing would be told a heading is a control. */
  .title:focus {
    outline: none;
  }

  .title {
    margin: 0;
    font-variation-settings: 'wght' 750, 'wdth' 108;
    font-size: 22px;
    line-height: 1.2;
    letter-spacing: -0.02em;
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
