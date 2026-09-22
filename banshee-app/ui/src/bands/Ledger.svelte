<script lang="ts">
  import { formatCount } from '../lib/history';
  import { findChord } from '../lib/keys';
  import SaveSwitch from '../controls/SaveSwitch.svelte';

  export let total: number;
  export let saving: boolean;
  export let open: () => void;
  export let find: () => void;
  export let id: string;

  // Read once: the chord names the platform's own key, which does not change
  // under this window.
  const chord = findChord();
</script>

<div class="ledger">
  <!-- The brief already calls this line the record's header. Saying so in the
       markup is what lets a screen reader jump to it. -->
  <h2>
    <button {id} class="state caps mono btn-underline" on:click={open}>
      {saving ? (total > 0 ? `${formatCount(total)} saved` : 'Nothing saved yet') : 'Not saving'}
      <span class="sr">— open what Banshee keeps</span>
    </button>
  </h2>

  <SaveSwitch {saving} />

  {#if saving && total > 0}
    <!-- A control, not a hint: the chord alone opens search for nobody who
         never presses it. The resting rule reads as the grip, the way the
         saving switch beside it does. -->
    <button
      id="ledger-find"
      class="find caps mono btn-underline"
      aria-label="Find in what was said ({chord})"
      on:click={find}
    >
      {chord} to find
    </button>
  {/if}
</div>

<style>
  .ledger {
    display: flex;
    align-items: center;
    gap: 16px;
    margin: 0 var(--gutter) 22px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--rule);
  }

  /* Flex, so the heading takes the button's height instead of adding a line
     box of its own and pushing the rule down. */
  h2 {
    display: flex;
    margin: 0;
    font: inherit;
  }

  /* The count opens a panel, so it rests with the same rule the two lesser
     controls beside it carry. */
  .state {
    color: var(--accent);
    border-bottom-color: currentcolor;
  }

  /* Lines up with the copy controls on the turns below. */
  .find {
    margin-left: auto;
    color: var(--dim);
    border-bottom-color: currentcolor;
  }
</style>
