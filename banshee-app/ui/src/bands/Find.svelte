<script lang="ts">
  import { land } from '../lib/focus';
  import { clearSearch, listed, view } from '../lib/history';
  import { listAndDetail } from '../lib/layout';

  let field: HTMLInputElement | undefined;
  let reveal: HTMLButtonElement | undefined;
  let revealed = false;

  $: shown = $listAndDetail || revealed || $view.query !== '';

  export async function open() {
    revealed = true;
    await land(() => field);
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    clearSearch();
    if ($listAndDetail) return;
    revealed = false;
    land(() => reveal);
  }

  function onBlur() {
    if ($view.query === '') revealed = false;
  }

  // Runs before the DOM swaps the field for the icon or back, so focus is read
  // while the control holding it still exists.
  function refit(wide: boolean) {
    if (wide && reveal && document.activeElement === reveal) land(() => field);
    else if (!wide && field && document.activeElement === field) revealed = true;
  }
  $: refit($listAndDetail);
</script>

{#snippet glass()}
  <svg class="icon" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
    <circle cx="7" cy="7" r="4.5" />
    <path d="M10.5 10.5 L14 14" />
  </svg>
{/snippet}

<div class="find">
  {#if shown}
    <label class="search">
      {@render glass()}
      <span class="sr">Search</span>
      <input
        bind:this={field}
        bind:value={$view.query}
        type="search"
        placeholder="Search"
        on:keydown={onKeydown}
        on:blur={onBlur}
      />
    </label>
  {:else}
    <button class="btn btn-ghost reveal" bind:this={reveal} on:click={open}>
      <span class="sr">Search</span>
      {@render glass()}
    </button>
  {/if}
  <span class="sr" aria-live="polite">
    {$view.query.trim() === '' ? '' : `${$listed.length} found`}
  </span>
</div>

<style>
  .find {
    display: flex;
    margin-left: auto;
    min-width: 0;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 220px;
    max-width: 100%;
    min-height: var(--control-height);
    padding: 5px 10px;
    border: 1px solid var(--dim);
    border-radius: var(--radius);
    color: var(--dim);
  }

  .search:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .search input:focus-visible {
    outline: none;
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 13px;
    line-height: 1.45;
  }

  input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }

  svg {
    flex: none;
    width: 16px;
    height: 16px;
  }

  .reveal {
    display: flex;
    padding: 7px;
  }
</style>
