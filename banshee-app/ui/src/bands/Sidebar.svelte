<script lang="ts">
  import Mark from '../marks/Mark.svelte';
  import HostLine from './HostLine.svelte';
  import { lampForm, type Word } from '../lib/daemon';
  import { MAIN, SETTINGS, type Page } from '../lib/nav';
  import { SIDEBAR } from './page';

  export let page: Page;
  export let go: (page: Page) => void;
  export let state: Word;
  /// Where audio goes while a remote listener is in use.
  export let host: string | null;
  /// The sentence each page's item says for a key the daemon has not applied.
  export let pending: Partial<Record<Page, string>>;
  export let warned: Page[];

  const GROUP = 'settings-heading';
  const NEEDS_ATTENTION = 'needs attention';
</script>

<!-- The spaces sit inside the hidden words: the accessible name joins these
     parts with none of its own. -->
{#snippet item(one: Page)}
  <li>
    <button class="item" aria-current={one === page ? 'page' : undefined} on:click={() => go(one)}>
      {one}
      {#if pending[one]}<span class="sr">{` ${pending[one]}`}</span>{/if}
      {#if warned.includes(one)}
        <span class="sr">{` ${NEEDS_ATTENTION}`}</span>
        <svg class="warn" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path
            fill="currentColor"
            fill-rule="evenodd"
            d="M8 1.5 L15 14 H1 Z M7.2 5.5 h1.6 v4.6 h-1.6 Z M7.05 12 a0.95 0.95 0 1 0 1.9 0 a0.95 0.95 0 1 0 -1.9 0 Z"
          />
        </svg>
      {/if}
    </button>
  </li>
{/snippet}

<nav class="sidebar" id={SIDEBAR} aria-label="Banshee">
  <ul class="items">
    {#each MAIN as one (one)}{@render item(one)}{/each}
  </ul>
  <span class="group" id={GROUP}>Settings</span>
  <div role="group" aria-labelledby={GROUP}>
    <ul class="items">
      {#each SETTINGS as one (one)}{@render item(one)}{/each}
    </ul>
  </div>

  <div class="foot">
    <p class="state"><span class="mark"><Mark form={lampForm(state)} /></span>{state}</p>
    {#if host !== null}
      <HostLine {host} />
    {/if}
  </div>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: 14px 10px;
    background: var(--foot);
    border-right: 1px solid var(--rule);
  }

  .items {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 30px;
    padding: 6px 10px;
    background: transparent;
    border: 0;
    border-radius: var(--radius);
    color: var(--ink);
    font-size: 13px;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }

  .item[aria-current='page'] {
    background: var(--hilite);
    font-weight: 700;
  }

  .warn {
    flex: none;
    width: 14px;
    height: 14px;
    margin-left: auto;
    color: var(--accent);
  }

  .group {
    display: block;
    padding: 20px 10px 4px;
    color: var(--dim);
    font-size: 13px;
    font-weight: 600;
  }

  .foot {
    display: grid;
    gap: 6px;
    margin-top: auto;
    padding: 12px 10px 2px;
    border-top: 1px solid var(--rule);
  }

  .foot p {
    margin: 0;
  }

  .state {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--ink);
    font-size: 13px;
    font-weight: 600;
  }

  .mark {
    display: flex;
    flex: none;
  }

  .mark :global(svg) {
    width: 15px;
    height: 15px;
  }
</style>
