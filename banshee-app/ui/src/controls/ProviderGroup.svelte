<script lang="ts">
  // One decision with dependents, so one group. Both panels use it, so the
  // listener and the speaker cannot drift apart in shape or in wording.
  import Row from './Row.svelte';
  import Segmented from './Segmented.svelte';

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
</script>

<div class="apart">
  <Row {name} block {note} {noteId}>
    <Segmented
      {label}
      {value}
      {options}
      describedBy={alsoId ? `${noteId} ${alsoId}` : noteId}
      {change}
    />
    <div class="parts" slot="under"><slot /></div>
  </Row>
</div>

<style>
  /* The 22px under the row above plus this padding is the 28px that holds the
     whole group away from the properties around it. */
  .apart {
    padding-top: 6px;
  }

  /* A panel that opens on the group has no row above, and the space would move
     its first label. */
  .apart:first-child {
    padding-top: 0;
  }

  .parts {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }
</style>
