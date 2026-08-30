<script lang="ts">
  // A job takes over the body rather than opening beside it, so the window
  // never has two things competing for the same attention, and so first run
  // has somewhere whole to land later.
  export let name: string;
  export let close: () => void;
</script>

<section class="panel" aria-label={name}>
  <div class="head">
    <h1 class="title">{name}</h1>
    <button class="close" on:click={close}>Done</button>
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

  .title {
    margin: 0;
    font-variation-settings: 'wght' 750, 'wdth' 108;
    font-size: 22px;
    line-height: 1.2;
    letter-spacing: -0.02em;
  }

  .close {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.12em;
    text-transform: uppercase;
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
