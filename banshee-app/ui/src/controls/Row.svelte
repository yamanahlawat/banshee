<script lang="ts">
  import { waiting } from '../lib/daemon';
  import { waitingSays } from '../lib/copy';
  import { refusalId, refused } from '../lib/settings';
  import Refusal from './Refusal.svelte';
  import { leave } from '../lib/travel';
  import { describeControls, idList } from '../lib/ids';

  export let name: string;
  export let note = '';
  /// Passed when something outside the row points at the note too.
  export let noteId: string | undefined = undefined;
  // The key this row writes, when it writes one.
  export let setting: string | undefined = undefined;
  // For a control with no baseline of its own, such as a range input, which
  // would otherwise drop its label to the bottom of the row.
  export let centred = false;
  /// The waiting line. Left out, it is the key's own while the daemon has not
  /// taken it. `null` when another row says it, or no row should. Text for a
  /// wait the daemon does not report, such as a chosen model that is absent.
  export let line: string | null | undefined = undefined;
  /// Off when another row says this key's refusal.
  export let refusal = true;
  /// On when the row goes because a move emptied it, so it fades with the move.
  export let leaves = false;

  $: waitingLine =
    line !== undefined
      ? line
      : setting !== undefined && $waiting.has(setting)
        ? waitingSays(setting)
        : null;
  $: refusedKey = refusal && setting !== undefined && $refused.has(setting) ? setting : undefined;
  $: ownNoteId = noteId ?? `note-${(setting ?? name).toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;
  $: describedBy = idList(note && ownNoteId, refusedKey && refusalId(refusedKey));
  const described = describeControls();
  $: described.set(describedBy);
</script>

<div class="row" class:centred out:leave={leaves}>
  <span class="name caps">{name}</span>
  <div class="value">
    <div class="control"><slot {describedBy} /></div>
    <!-- Both, and the note first: a pending line that replaces the note drops
         what the control means at the moment the reader changed it. -->
    {#if note}
      <p class="note" id={ownNoteId}>{note}</p>
    {/if}
    {#if waitingLine !== null}
      <p class="note pending">{waitingLine}</p>
    {/if}
    {#if refusedKey}<Refusal setting={refusedKey} />{/if}
    <slot name="under" />
    <!-- After the sentence that explains it, the way a drawn box orders its own
         head, consequence and fix. -->
    <slot name="action" />
  </div>
</div>

<style>
  /* Separation by void, never a rule between properties. A name too long for
     the column wraps, and its first line keeps the control's baseline. */
  .row {
    display: grid;
    grid-template-columns: var(--label-column) 1fr;
    column-gap: var(--label-gap);
    align-items: baseline;
    margin-bottom: 22px;
  }

  .centred {
    align-items: center;
  }

  .name {
    color: var(--accent);
  }

  .value {
    min-width: 0;
  }

  .note {
    margin: 8px 0 0;
  }
</style>
