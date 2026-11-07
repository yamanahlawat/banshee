<script lang="ts">
  import { copied, copy } from '../lib/copy';
  import { dayLabel, rowFlag } from '../lib/history';
  import type { HistoryRow } from '../lib/tauri';
  import { formatTime } from '../lib/time';
  import Marked from './Marked.svelte';

  export let row: HistoryRow;
  export let now: Date;
  export let goListening: () => void;
  /// The trimmed search, whose matches the words mark.
  export let query = '';

  let heading: HTMLElement;

  export function focus() {
    heading?.focus();
  }

  $: flag = rowFlag(row);
  $: failedDictation = row.kind === 'dictation' && row.outcome === 'failed';
  $: note =
    row.kind === 'dictation'
      ? null
      : row.outcome === 'failed'
        ? row.reason
        : row.outcome === 'unasked' && row.reason
          ? `The agent was told: ${row.reason}`
          : null;
</script>

<div class="detail" role="region" aria-label="Selected row">
  <h2 class="when" tabindex="-1" bind:this={heading}>
    {dayLabel(row.timestamp, now)} at <span class="mono time">{formatTime(row.timestamp)}</span>
  </h2>

  {#snippet copyButton(label: string, text: string, which: string)}
    <button class="btn btn-ghost copy" on:click={() => copy(text, `${row.id}:${which}`)}>
      <span>{$copied === `${row.id}:${which}` ? 'Copied' : label}</span>
      <!-- Holds the button at the wider of its two labels. -->
      <span class="held" aria-hidden="true"
        >{label.length > 'Copied'.length ? label : 'Copied'}</span
      >
    </button>
  {/snippet}

  {#if failedDictation}
    <p class="words failed">Transcription failed</p>
    {#if flag}<p class="note">{flag}</p>{/if}
    <div class="actions">
      <button class="btn" on:click={goListening}>Open Listening</button>
    </div>
  {:else if row.kind === 'dictation'}
    <p class="words user"><Marked text={row.text ?? ''} needle={query} /></p>
    <div class="actions">{@render copyButton('Copy', row.text ?? '', 'text')}</div>
  {:else}
    <p class="words agent"><Marked text={row.question ?? ''} needle={query} /></p>
    <hr />
    {#if row.outcome === 'transcribed'}
      <p class="words user"><Marked text={row.text ?? ''} needle={query} /></p>
    {:else}
      {#if flag}<p class="flag">{flag}</p>{/if}
      {#if note}<p class="note">{note}</p>{/if}
    {/if}
    <div class="actions">
      {#if row.outcome === 'transcribed'}
        {@render copyButton('Copy answer', row.text ?? '', 'answer')}
      {/if}
      {@render copyButton('Copy question', row.question ?? '', 'question')}
    </div>
  {/if}
</div>

<style>
  .detail {
    min-width: 0;
    height: 100%;
    overflow-y: auto;
    padding: 44px 52px;
  }
  .detail > * {
    max-width: 580px;
  }
  .when {
    margin: 0 0 20px;
    font-size: 13px;
    line-height: 1.45;
    font-weight: 400;
    color: var(--dim);
  }
  .when:focus-visible {
    outline-offset: 4px;
  }
  .time {
    color: var(--accent);
  }
  .words {
    margin: 0;
    font-size: 22px;
    line-height: 1.3;
    letter-spacing: -0.01em;
    overflow-wrap: anywhere;
    text-wrap: pretty;
  }
  .user {
    font-variation-settings:
      'wght' var(--cut-user-weight),
      'wdth' var(--cut-user-width);
  }
  .agent {
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
  }
  .failed {
    font-variation-settings:
      'wght' 700,
      'wdth' 100;
  }
  hr {
    margin: 22px 0;
    border: 0;
    border-top: 1px solid var(--rule);
  }
  .flag {
    margin: 0;
    font-size: 17px;
    line-height: 1.3;
    font-variation-settings:
      'wght' 650,
      'wdth' var(--cut-agent-width);
    color: var(--accent);
  }
  .note {
    margin: 8px 0 0;
    max-width: 460px;
    color: var(--dim);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 24px;
  }
  .copy {
    display: inline-grid;
  }
  .copy > * {
    grid-area: 1 / 1;
  }
  .held {
    visibility: hidden;
  }
</style>
