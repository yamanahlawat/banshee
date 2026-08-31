<script lang="ts">
  export let query = '';
  export let matches = 0;
  export let close: () => void;

  let field: HTMLInputElement | undefined;

  // Revealed by a keystroke, so it has to take the caret with it. Nobody
  // presses a shortcut and then reaches for the mouse.
  $: if (field) field.focus();
</script>

<div class="find">
  <input
    bind:this={field}
    bind:value={query}
    class="field mono"
    type="search"
    placeholder="Find in what was said"
    aria-label="Find in what was said"
  />
  <span class="readout" aria-live="polite">
    {query.trim() === '' ? '' : `${matches} found`}
  </span>
  <button class="dismiss caps" aria-label="Close find" on:click={close}>Esc</button>
</div>

<style>
  .find {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0 var(--gutter) 22px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--accent);
  }

  .field {
    flex: 1;
    min-width: 0;
    /* A pointer's height, stated rather than derived: padding plus the font's
       own line box came to 23.55px, which reads as 24 in any rounded output and
       is not. */
    min-height: 24px;
    padding: 3px 0;
    font-size: 13px;
    color: var(--ink);
    background: transparent;
    border: 0;
    -webkit-appearance: none;
    appearance: none;
  }

  .field::placeholder {
    color: var(--dim);
  }

  .field::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }

  .dismiss {
    color: var(--ink);
    background: transparent;
    border: 1px solid var(--rule);
    border-radius: 0;
    padding: 4px 8px;
    cursor: pointer;
    flex: none;
  }

  .dismiss:hover {
    border-color: var(--ink);
  }
</style>
