<script lang="ts">
  // Write-only. The daemon says whether a key is set and never what it is, so
  // this row draws presence and the two acts on it, in one place for both sides.
  import { tick } from 'svelte';
  import { write } from '../lib/settings';
  import { announce } from '../lib/copy';
  import Field from './Field.svelte';
  import SubRow from './SubRow.svelte';

  export let setting: string;
  export let present: boolean;

  // The daemon reads an empty key as removal.
  async function removeKey() {
    if (await write(setting, '')) {
      announce('The key is removed. It takes effect when Banshee restarts.');
    }
  }

  // The key it holds cannot be shown, so replacing one is a field that arrives
  // where the line saying it is set stood.
  let replacing = false;
  let field: HTMLInputElement | undefined;
  async function replaceKey() {
    replacing = true;
    await tick();
    field?.focus();
  }
</script>

<SubRow name="key">
  {#if present && !replacing}
    <span class="held">A key is set</span>
    <button class="caps btn-underline act" aria-label="Remove the key" on:click={removeKey}>
      Remove
    </button>
    <button class="caps btn-underline act" aria-label="Replace the key" on:click={replaceKey}>
      Replace
    </button>
  {:else}
    <Field
      bind:input={field}
      label="Key"
      masked
      dashed={!present}
      placeholder="Paste a key"
      cancel={() => (replacing = false)}
      commit={(next) => {
        replacing = false;
        return next === '' ? undefined : write(setting, next);
      }}
    />
  {/if}
</SubRow>

<style>
  /* A resting rule, or an 11px word beside a field reads as part of the value. */
  .act {
    color: var(--ink);
    border-bottom-color: currentcolor;
  }

  /* The field's own box, so the row's baseline and its edge stay put when the
     line and the field swap places. */
  .held {
    font-variation-settings:
      'wght' 500,
      'wdth' 100;
    font-size: 15px;
    padding: 6px 0 7px;
  }
</style>
