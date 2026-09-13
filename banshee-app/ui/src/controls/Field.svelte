<script lang="ts">
  import { onDestroy } from 'svelte';
  import { claimKeys } from '../lib/keys';

  export let label: string;
  export let value = '';
  export let placeholder = '';
  export let masked = false;
  /// The world's mark for a thing that is not there, for a field whose value is
  /// missing rather than merely empty.
  export let dashed = false;
  /// `false` means the daemon refused it. Anything else is taken.
  export let commit: (next: string) => boolean | void | Promise<boolean | void>;
  /// Called when Escape leaves the field, for a caller that opened it.
  export let cancel: (() => void) | undefined = undefined;
  // Bound by a caller that has to focus a field it just swapped in.
  export let input: HTMLInputElement | undefined = undefined;

  let draft = value;
  $: draft = value;
  let release: (() => void) | null = null;
  // A write refused after a newer one landed skips its rollback.
  let generation = 0;

  // Typing owns the keyboard: otherwise Escape closes the panel and Cmd+F opens
  // Find mid-word.
  function take() {
    release ??= claimKeys();
  }

  function letGo() {
    release?.();
    release = null;
  }

  // A sent value is the resting value, or the next focus and blur sends it
  // again. A masked field rests empty: the daemon takes a key and answers only
  // whether one is set, so there is nothing to hold.
  async function settle() {
    letGo();
    if (draft === value) return;
    const mine = ++generation;
    const sent = draft;
    const stored = value;
    draft = masked ? '' : sent;
    value = draft;
    if ((await commit(sent)) === false && mine === generation) {
      value = stored;
      draft = value;
    }
  }

  function onKeydown(event: KeyboardEvent) {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === 'Enter') {
      settle();
      input.blur();
    }
    if (event.key === 'Escape') {
      event.stopPropagation();
      draft = value;
      letGo();
      input.blur();
      cancel?.();
    }
  }

  // A field that leaves with the row it sits in never blurs.
  onDestroy(letGo);
</script>

<input
  bind:this={input}
  class="field"
  class:dashed
  type={masked ? 'password' : 'text'}
  aria-label={label}
  autocomplete={masked ? 'off' : undefined}
  {placeholder}
  value={draft}
  on:input={(e) => (draft = e.currentTarget.value)}
  on:focus={take}
  on:blur={settle}
  on:keydown={onKeydown}
/>

<style>
  .field {
    font-variation-settings:
      'wght' 500,
      'wdth' 100;
    font-size: 15px;
    border-bottom: 1px solid var(--ink);
    border-radius: 0;
    padding: 6px 0;
    margin: 0;
  }

  /* A dashed accent line is what this world draws around a thing that is not
     there, and an underline is the form of it a control can carry. */
  .dashed {
    border-bottom-style: dashed;
    border-bottom-color: var(--accent);
  }
</style>
