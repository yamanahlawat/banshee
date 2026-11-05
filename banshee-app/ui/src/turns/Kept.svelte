<script lang="ts">
  import type { HistoryRow } from '../lib/tauri';
  import Turn from './Turn.svelte';

  export let row: HistoryRow;
  export let time: string;
  export let lead: boolean;
  export let arrive: boolean;

  const id = String(row.id);

  const withReason = (line: string, reason: string | null) =>
    reason ? `${line}: ${reason}` : `${line}.`;

  const QUESTION_NOTES: Partial<Record<HistoryRow['outcome'], (reason: string | null) => string>> =
    {
      no_answer: () => 'No answer heard.',
      closed: () => 'Closed before you answered.',
      failed: (reason) => withReason('Not transcribed', reason),
      unasked: (reason) => withReason('Not asked aloud', reason),
    };

  $: note = QUESTION_NOTES[row.outcome]?.(row.reason) ?? '';
  $: answered = row.kind === 'question' && row.outcome === 'transcribed';
</script>

{#if row.kind === 'dictation'}
  {#if row.outcome === 'failed'}
    <Turn {lead} {arrive} speaker="user" {time} text="Transcription failed">
      {#if row.reason}<p class="note">{row.reason}</p>{/if}
    </Turn>
  {:else}
    <Turn {lead} {arrive} speaker="user" {id} {time} text={row.text ?? ''} />
  {/if}
{:else}
  <Turn
    lead={lead && !answered}
    {arrive}
    speaker="agent"
    id="{id}-question"
    {time}
    said="Agent asked"
    text={row.question ?? ''}
  >
    {#if note}<p class="note">{note}</p>{/if}
  </Turn>
  {#if answered}
    <Turn
      {lead}
      {arrive}
      speaker="user"
      id="{id}-answer"
      {time}
      said="You answered"
      text={row.text ?? ''}
    />
  {/if}
{/if}

<style>
  .note {
    margin: 6px 0 0;
  }
</style>
