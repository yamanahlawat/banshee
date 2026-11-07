<script lang="ts">
  import { getContext, tick } from 'svelte';
  import { land } from '../lib/focus';
  import {
    forget,
    formatCount,
    keptAudio,
    readAfter,
    readAll,
    table,
    wouldDelete,
  } from '../lib/history';
  import { PAGE, type PageFocus } from '../bands/page';
  import { clearHistory, type HistoryRow } from '../lib/tauri';
  import { announce, report } from '../lib/copy';
  import { daemon } from '../lib/daemon';
  import { defaultOf, optionsOf, table as settings, write } from '../lib/settings';
  import SaveSwitch from '../controls/SaveSwitch.svelte';
  import Row from '../controls/Row.svelte';
  import Segmented from '../controls/Segmented.svelte';

  // Deleting is not undoable and not reversible from anywhere else, so it
  // asks once before it acts.
  let confirming = false;

  export let saving: boolean;
  $: total = $table.total;

  const page = getContext<PageFocus>(PAGE);

  let keep: HTMLButtonElement;
  let clearer: HTMLButtonElement;
  let deleting = false;

  function clearSays(rows: HistoryRow[], count: number): string {
    const now = new Date();
    const audio = rows.some((row) => keptAudio(row, now) !== null) ? ' and their audio' : '';
    const noun = count === 1 ? 'row' : 'rows';
    return `Delete all ${formatCount(count)} ${noun}${audio}? This cannot be undone.`;
  }

  $: daemonConfig = ($daemon.status?.config?.daemon ?? {}) as Record<string, unknown>;
  $: keepAudio = String(daemonConfig.keep_audio ?? defaultOf($settings, 'daemon.keep_audio'));
  $: periods = optionsOf($settings, 'daemon.keep_audio');
  const KEEP_AUDIO_NOTE = 'keep-audio-note';
  const KEEP_AUDIO_QUESTION = 'keep-audio-question';
  const CLEAR_QUESTION = 'clear-question';

  // The period just written, shown until the daemon's answer lands, so the
  // control's stop and check sit where focus already is.
  let chosen: string | null = null;
  $: shown = chosen ?? keepAudio;

  // The period a Delete would write, and the recordings it would take, while
  // the question shows.
  let shortening: { period: string; count: number } | null = null;
  let keepPeriod: HTMLButtonElement;
  let periodChoice: Segmented | undefined;

  async function landOnPeriod(period: string) {
    await tick();
    periodChoice?.focus(period);
  }

  function shortenSays(period: string, count: number): string {
    const what =
      period === 'off'
        ? 'all kept audio'
        : `audio older than ${periods.find((option) => option.value === period)?.label}`;
    const recordings = `${formatCount(count)} ${count === 1 ? 'recording' : 'recordings'}`;
    return `Delete ${what}? ${recordings}. This cannot be undone.`;
  }

  async function keepFor(period: string) {
    chosen = period;
    const landed = await write('daemon.keep_audio', period);
    chosen = null;
    if (landed) {
      // Each row's `until` comes from the daemon, so the rows read the new
      // period only once it has landed.
      readAfter().catch(() => {});
    } else {
      landOnPeriod(keepAudio);
    }
  }

  async function choosePeriod(period: string) {
    if (period === shown) return;
    // An unread table holds no rows, and a count of none would skip the question.
    if (!$table.loaded) await readAll().catch(() => {});
    const count = wouldDelete($table.rows, period, new Date());
    if (count === 0) {
      keepFor(period);
      return;
    }
    shortening = { period, count };
    land(() => keepPeriod);
  }

  function keepTheAudio() {
    shortening = null;
    landOnPeriod(keepAudio);
  }

  async function shorten() {
    // Svelte removes Delete on the next tick, and a second press that lands
    // before then still reaches this handler.
    if (shortening === null) return;
    const { period } = shortening;
    shortening = null;
    chosen = period;
    await landOnPeriod(period);
    await keepFor(period);
  }

  async function show(next: boolean, on: () => HTMLElement | undefined) {
    confirming = next;
    return land(on);
  }

  /// Not `disabled`: `clear` closes the question and puts focus back on this
  /// button, and a disabled one cannot hold it.
  function ask() {
    if (deleting) return;
    show(true, () => keep);
  }

  // `confirming` closes first: the round trip is not instant, and a second
  // press on a live Delete would run the whole thing again.
  async function clear() {
    // `disabled` is not enough: Svelte removes this button on the next tick, and
    // a press that landed before then still reaches the handler on the node.
    if (deleting) return;
    deleting = true;
    await show(false, () => clearer);
    try {
      await clearHistory();
      forget();
      // A delete that works takes Clear with it, so the page says where next.
      page?.refocus();
      announce('History cleared.');
    } catch {
      report('The record could not be cleared. Nothing was deleted.');
    }
    deleting = false;
  }
</script>

<div class="record">
  {#if confirming}
    <!-- On one line with the buttons this wraps at 480 and drops the
         destructive control alone at the gutter, which is the strongest
         reading position on the page. -->
    <p class="warn" id={CLEAR_QUESTION}>{clearSays($table.rows, total)}</p>
    <div class="actions">
      <button
        bind:this={keep}
        class="btn"
        aria-describedby={CLEAR_QUESTION}
        on:click={() => show(false, () => clearer)}
      >
        Keep it
      </button>
      <button
        class="btn btn-ghost"
        aria-describedby={CLEAR_QUESTION}
        disabled={deleting}
        on:click={clear}>Delete everything</button
      >
    </div>
  {:else}
    <div class="actions">
      <SaveSwitch {saving} />
      {#if total > 0}
        <button bind:this={clearer} class="btn btn-ghost" on:click={ask}> Clear </button>
      {/if}
    </div>
  {/if}

  <p class="note">It lives in a file only you can read. Nothing is sent anywhere.</p>

  {#if !saving}
    <p class="note">Banshee is not keeping anything new. What it kept stays until you clear it.</p>
  {/if}
</div>

<div class="keep">
  <Row
    name="Keep audio"
    block
    note="Audio holds whatever the microphone heard, other voices too. A shorter period deletes older audio."
    noteId={KEEP_AUDIO_NOTE}
    setting="daemon.keep_audio"
  >
    {#if shortening}
      <!-- One block, so the row's control line does not set the question beside its buttons. -->
      <div>
        <p class="warn" id={KEEP_AUDIO_QUESTION}>
          {shortenSays(shortening.period, shortening.count)}
        </p>
        <div class="actions">
          <button
            bind:this={keepPeriod}
            class="btn"
            aria-describedby={KEEP_AUDIO_QUESTION}
            on:click={keepTheAudio}>Keep it</button
          >
          <button class="btn btn-ghost" aria-describedby={KEEP_AUDIO_QUESTION} on:click={shorten}
            >Delete</button
          >
        </div>
      </div>
    {:else}
      <Segmented
        bind:this={periodChoice}
        label="Keep audio"
        value={shown}
        options={periods}
        describedBy={KEEP_AUDIO_NOTE}
        change={choosePeriod}
      />
    {/if}
  </Row>
</div>

<style>
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
  }

  .warn {
    max-width: 520px;
    margin: 0 0 12px;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    color: var(--accent);
  }

  .note {
    margin: 12px 0 0;
  }

  .keep {
    margin-top: 22px;
  }
</style>
