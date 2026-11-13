<script lang="ts">
  import { announce, asSentence, copied, copy, report, serverHost } from '../lib/copy';
  import { daemon, listeningFacts } from '../lib/daemon';
  import {
    asker,
    dayLabel,
    keptAudio,
    keptUntil,
    replaceRow,
    retrying,
    rowFlag,
  } from '../lib/history';
  import { retry, type HistoryRow } from '../lib/tauri';
  import { formatTime } from '../lib/time';
  import Marked from './Marked.svelte';
  import Player from './Player.svelte';

  export let row: HistoryRow;
  export let now: Date;
  export let goListening: () => void;
  /// The trimmed search, whose matches the words mark.
  export let query = '';

  let heading: HTMLElement;
  let player: Player | undefined;

  export function focus() {
    heading?.focus();
  }

  export function togglePlay() {
    player?.toggle();
  }

  const wider = (one: string, other: string) => (one.length > other.length ? one : other);

  $: flag = rowFlag(row);
  $: failedDictation = row.kind === 'dictation' && row.outcome === 'failed';
  // A fresh clock per row: `now` moves only when the day does.
  $: audio = keptAudio(row, new Date());
  $: retryable = audio !== null && row.kind === 'dictation';
  $: busy = $retrying.has(row.id);
  $: listening = listeningFacts($daemon);
  $: model = $daemon.status?.stt_models?.find((one) => one.id === $daemon.status?.stt_model);
  $: retryLabel = listening.remote
    ? 'Retry with the transcription server'
    : model
      ? `Retry with ${model.label}`
      : 'Retry';
  // Read off the listener in force, not off dictation working: a retry goes
  // to the server even while the microphone is broken.
  $: sendsTo = listening.remote ? serverHost(listening) : null;

  async function retryRow() {
    const was = row;
    if ($retrying.has(was.id)) return;
    retrying.update((ids) => new Set(ids).add(was.id));
    try {
      replaceRow(await retry(was.id));
      announce(was.outcome === 'failed' ? 'Transcribed.' : 'Text replaced.');
    } catch (error) {
      const reason = (error as { message?: string })?.message?.trim();
      // A failed row has no text to keep.
      const kept = was.outcome === 'failed' ? null : 'The text is unchanged.';
      report(['The retry failed.', reason && asSentence(reason), kept].filter(Boolean).join(' '));
    } finally {
      retrying.update((ids) => new Set([...ids].filter((id) => id !== was.id)));
    }
  }

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
    {dayLabel(row.timestamp, now)} at
    <span class="mono time">{formatTime(row.timestamp)}</span>
  </h2>

  {#snippet labelled(shown: string, widest: string)}
    <span>{shown}</span>
    <!-- Holds the button at the wider of its two labels. -->
    <span class="held" aria-hidden="true">{widest}</span>
  {/snippet}

  {#snippet copyButton(label: string, text: string, which: string)}
    <button class="btn btn-ghost holds" on:click={() => copy(text, `${row.id}:${which}`)}>
      {@render labelled(
        $copied === `${row.id}:${which}` ? 'Copied' : label,
        wider(label, 'Copied'),
      )}
    </button>
  {/snippet}

  {#if failedDictation}
    <p class="words failed" class:pending={busy}>Transcription failed</p>
    {#if flag}<p class="note">{flag}</p>{/if}
  {:else if row.kind === 'dictation'}
    <p class="words user" class:pending={busy}><Marked text={row.text ?? ''} needle={query} /></p>
  {:else}
    <p class="caps caption">{asker(row)} asked</p>
    <p class="words agent"><Marked text={row.question ?? ''} needle={query} /></p>
    <hr />
    {#if row.outcome === 'transcribed'}
      <p class="caps caption">You answered</p>
      <p class="words user"><Marked text={row.text ?? ''} needle={query} /></p>
    {:else}
      {#if flag}<p class="flag">{flag}</p>{/if}
      {#if note}<p class="note">{note}</p>{/if}
    {/if}
  {/if}
  {#if audio}
    {#key row.id}
      <Player bind:this={player} id={row.id} seconds={audio.seconds} />
    {/key}
    <p class="kept">{keptUntil(audio.until)}</p>
  {/if}
  {#if failedDictation && !audio}
    <p class="kept">No audio was kept, so there is nothing to retry.</p>
  {/if}
  <!-- One row for every kind, so Retry stays the same element when a failed
       dictation turns into a transcribed one, and focus stays on it. -->
  <div class="actions">
    {#if row.kind === 'question'}
      {#if row.outcome === 'transcribed'}
        {@render copyButton('Copy answer', row.text ?? '', 'answer')}
      {/if}
      {@render copyButton('Copy question', row.question ?? '', 'question')}
    {:else if !failedDictation}
      {@render copyButton('Copy', row.text ?? '', 'text')}
    {/if}
    {#if retryable}
      <button
        class="btn holds"
        class:btn-ghost={!failedDictation}
        aria-describedby={sendsTo ? 'retry-sends' : undefined}
        on:click={retryRow}
      >
        {@render labelled(busy ? 'Retrying' : retryLabel, wider(retryLabel, 'Retrying'))}
      </button>
    {/if}
    {#if failedDictation}
      <button class="btn" class:btn-ghost={retryable} on:click={goListening}>Open Listening</button>
    {/if}
  </div>
  {#if retryable && sendsTo}
    <p class="kept" id="retry-sends">Sends this recording to {sendsTo}.</p>
  {/if}
</div>

<style>
  .detail {
    min-width: 0;
    height: 100%;
    overflow-y: auto;
    padding: 44px 52px;
  }
  @media (max-width: 859.98px) {
    .detail {
      padding: 22px var(--gutter);
    }
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
  .caption {
    margin: 0 0 8px;
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .words {
    margin: 0;
    font-size: 22px;
    line-height: 1.3;
    letter-spacing: -0.01em;
    overflow-wrap: anywhere;
    text-wrap: pretty;
  }
  .words.pending {
    color: var(--dim);
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
  .kept {
    margin: 8px 0 0;
    font-size: 13px;
    line-height: 1.45;
    color: var(--dim);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 24px;
  }
  .holds {
    display: inline-grid;
  }
  .holds > * {
    grid-area: 1 / 1;
  }
  .held {
    visibility: hidden;
  }
</style>
