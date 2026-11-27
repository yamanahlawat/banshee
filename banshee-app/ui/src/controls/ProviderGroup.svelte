<script lang="ts">
  // One decision with dependents, so one group. Both pages use it, so the
  // listener and the speaker cannot drift apart in shape or in wording.
  import Row from './Row.svelte';
  import Segmented from './Segmented.svelte';
  import { idList } from '../lib/ids';

  export let name: string;
  export let label: string;
  export let value: string;
  export let options: { value: string; label: string }[];
  // The sentence that says what the choice does. The radiogroup is described
  // by it, so the group is read with its consequence.
  export let note: string;
  export let noteId: string;
  // A second line the slot draws under the parts, read with the group when it
  // is there. It arrives with its own content, so nothing announces it.
  export let alsoId: string | undefined = undefined;
  export let change: (next: string) => void;
  /// The provider key, whose refusal the switch's row says. The note already
  /// says when the daemon has not taken it, so no pending line joins it.
  export let setting: string;
</script>

<div class="apart">
  <Row {name} {note} {noteId} {setting} line={null} let:describedBy>
    <Segmented {label} {value} {options} describedBy={idList(describedBy, alsoId)} {change} />
  </Row>
  <slot />
</div>

<style>
  /* The parts sit closer to the switch and to each other than rows elsewhere
     on the page, so the group reads as one decision. */
  .apart {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 22px;
  }

  .apart > :global(.row) {
    margin-bottom: 0;
  }
</style>
