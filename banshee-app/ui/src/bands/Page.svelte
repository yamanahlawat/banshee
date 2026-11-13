<script lang="ts">
  import { setContext } from 'svelte';
  import { PAGE, PAGE_HEADING } from './page';

  export let name: string;
  /// What is true of this page's job right now, read from the daemon. It is the
  /// heading, because a heading may not be smaller than the text it heads and
  /// the statement is the largest thing here. History has none, so its title
  /// heads it.
  export let lead: string | null = null;
  /// The page's scrolling region, which the window returns to its top on each page change.
  export let scroller: HTMLElement | undefined = undefined;
  /// The page scrolls in regions of its own, so the page as a whole does not.
  export let regions = false;
  /// False while the toolbar's own Back button already names the page. The title
  /// stays in the DOM, since the page's heading is where focus returns.
  export let titled = true;

  const TITLE = 'page-title';

  setContext(PAGE, { refocus: () => document.getElementById(PAGE_HEADING)?.focus() });
</script>

<section class="page" aria-labelledby={lead === null ? PAGE_HEADING : TITLE}>
  <div class="toolbar">
    <slot name="start" />
    {#if lead === null}
      <h2 class="title" class:sr={!titled} id={PAGE_HEADING} tabindex="-1">{name}</h2>
    {:else}
      <p class="title" class:sr={!titled} id={TITLE}>{name}</p>
    {/if}
    <slot name="end" />
  </div>
  <div class="scroller" class:regions bind:this={scroller}>
    <slot name="notice" />
    <div class:column={lead !== null} class:fill={regions}>
      {#if lead !== null}
        <h2 class="statement" id={PAGE_HEADING} tabindex="-1">{lead}</h2>
      {/if}
      <slot />
    </div>
  </div>
</section>

<style>
  .page {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 14px;
    flex: none;
    min-height: 56px;
    padding: 0 var(--gutter);
    border-bottom: 1px solid var(--rule);
  }

  .title:not(.sr) {
    margin: 0;
  }

  .title {
    font-size: 17px;
    line-height: 1.3;
    letter-spacing: -0.01em;
    font-variation-settings:
      'wght' var(--cut-head-weight),
      'wdth' var(--cut-head-width);
  }

  /* The page scrolls below the toolbar, so the toolbar stays put however long
     the page runs. */
  .scroller {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-top: 22px;
  }

  .scroller.regions {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding-top: 0;
  }

  .fill {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .column {
    max-width: var(--measure);
    box-sizing: content-box;
    padding: 0 var(--gutter) 24px;
  }

  /* No local focus rule on purpose: the global ring marks keyboard entry, and
     :focus-visible keeps it off for pointer users. */

  /* The lead treatment, in the agent cut: Banshee stating what is true of
     itself, in the same voice it uses for everything it says. */
  .statement {
    margin: 0 0 26px;
    max-width: var(--measure);
    text-wrap: balance;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 28px;
    line-height: 1.3;
    letter-spacing: -0.03em;
  }
</style>
