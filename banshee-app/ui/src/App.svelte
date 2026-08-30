<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    daemon,
    downloadLine,
    endsTheRun,
    lampForm,
    microphoneInUse,
    reduceLive,
    reduceStatus,
    waitsOnARestart,
    stateWord,
    type Live,
    type Status,
  } from './lib/daemon';
  import { announce, announcement } from './lib/copy';
  import { followSaveHistory, readAll, readLatest, readNewest, table } from './lib/history';
  import { agents, refresh as readAgents } from './lib/agents';
  import {
    listen,
    listVoices,
    restartDaemon,
    startDaemon,
    status,
    type Down,
    type DownloadProgress,
    type Voices,
  } from './lib/tauri';
  import { humanize } from './lib/hotkey';
  import { keysClaimed } from './lib/keys';
  import { formatWhen } from './lib/time';
  import Header from './bands/Header.svelte';
  import Foot from './bands/Foot.svelte';
  import Panel from './bands/Panel.svelte';
  import Blockers from './bands/Blockers.svelte';
  import Find from './bands/Find.svelte';
  import Ledger from './bands/Ledger.svelte';
  import TheRecord from './bands/Record.svelte';
  import Turn from './turns/Turn.svelte';
  import Absence from './turns/Absence.svelte';
  import Pending from './turns/Pending.svelte';
  import MicrophonePanel from './jobs/MicrophonePanel.svelte';
  import HotkeyPanel from './jobs/HotkeyPanel.svelte';
  import VoicePanel from './jobs/VoicePanel.svelte';
  import AgentsPanel from './jobs/AgentsPanel.svelte';

  let wasTranscribing = false;
  let voices: Voices = { voices: [], current: null };
  let voicesRead = false;
  let agentsRead = false;
  type Job = 'Microphone' | 'Hotkey' | 'Voice' | 'Agents' | 'Record';
  let job: Job | null = null;

  // A panel takes over the body, so an opener standing in that body is
  // destroyed by the click it is answering. The way back is the opener's id
  // and never the node, which no longer exists by the time it is wanted.
  const RETURNS_TO = { ledger: 'ledger', absence: 'nothing-yet' };
  let cameFrom = '';

  async function openJob(next: Job | null, from = '') {
    cameFrom = next === null ? cameFrom : from;
    job = next;
    if (next !== null) return;
    await tick();
    document.getElementById(cameFrom)?.focus();
    cameFrom = '';
  }
  let query = '';
  let finding = false;
  // One instant for the whole paint, so two rows cannot straddle midnight.
  $: now = ($table.rows, new Date());
  $: rightNow = formatWhen(now.toISOString(), now);

  type Live_ = 'recording' | 'transcribing' | null;

  // Speaking is absent deliberately: the daemon stores nothing Banshee says, so
  // a caret would hold a place no turn ever fills.
  function pendingOf(live: Live): Live_ {
    if (live.transcribing) return 'transcribing';
    if (live.recording) return 'recording';
    return null;
  }
  // The record runs to thousands of rows. Find still reads every one of them.
  const PAGE = 40;
  let limit = PAGE;

  function onKeydown(event: KeyboardEvent) {
    if (keysClaimed()) return;
    if ((event.metaKey || event.ctrlKey) && event.key === 'f') {
      event.preventDefault();
      openJob(null);
      finding = true;
      return;
    }
    if (event.key !== 'Escape') return;
    if (job !== null) openJob(null);
    else if (finding) closeFind();
  }

  function closeFind() {
    finding = false;
    query = '';
  }

  $: word = stateWord($daemon);
  $: form = lampForm(word);
  $: config = ($daemon.status?.config ?? {}) as Record<string, Record<string, unknown>>;
  $: live = word !== 'Not running';
  $: connected = $agents.filter((a) => a.presence === 'connected').length;

  $: rows = $table.rows;
  $: needle = query.trim().toLowerCase();
  $: shown = needle ? rows.filter((r) => r.text.toLowerCase().includes(needle)) : rows;
  $: needle, (limit = PAGE);
  $: visible = shown.slice(0, limit);
  $: older = shown.length - visible.length;
  // The daemon sets `armed` from `ask_user` alone, and sends no question with
  // it, so the turn can say an agent is waiting and nothing further.
  $: waiting = $daemon.live.armed;
  $: pending = waiting ? null : pendingOf($daemon.live);
  $: nothingYet = $table.loaded && rows.length === 0;
  $: blockers = $daemon.status?.blockers ?? [];
  // The daemon answers this, and the config only says what was asked for. A
  // history file that will not open leaves the two disagreeing.
  $: savingHistory = ($daemon.status?.history_enabled ?? config.daemon?.save_history) !== false;

  // A setting the daemon could not apply live needs the process replaced, and
  // the window can do that rather than name a command for a terminal.
  let restarting = false;
  const COMING_BACK = 12;
  async function restart() {
    restarting = true;
    try {
      await restartDaemon();
      // `launchctl kickstart -k` answers when launchd accepts the request, not
      // when the new process listens. Reading straight away finds a dead socket
      // and tells the user Banshee has stopped, which is the opposite of what
      // just happened. Nothing measured this count; it trades how slow a
      // machine may be against how long the window waits before believing it.
      for (let left = COMING_BACK; left > 0; left--) {
        if (await readStatus()) break;
        await new Promise((wake) => setTimeout(wake, 250));
      }
      await readLatest();
    } catch {
      // The pending keys are still pending, so the notice already says it.
    }
    restarting = false;
  }

  let starting = false;
  async function start() {
    starting = true;
    try {
      await startDaemon();
      await readDaemon();
    } catch {
      // The state word still says Not running, so the failure is already said.
    }
    starting = false;
  }

  // A stopped daemon is acting on none of these, so each reads empty.
  //
  // The hotkey is the config's, because the daemon reports no bound key, and it
  // is never applied live. Marking it pending is the only way the foot can stop
  // naming a key the daemon is not listening for. `audio.input_device` always
  // applies, so the microphone has no such state.
  $: footValues = ((): { id: string; label: Job; value: string; pending?: boolean }[] => {
    const voice = String(config.tts?.voice ?? '');
    const said = (value: string) => (live ? value : '');
    const waits = (...keys: string[]) => live && keys.some((key) => $waitsOnARestart.has(key));
    return [
      { id: 'job-microphone', label: 'Microphone', value: said(microphoneInUse($daemon.live.audio_device)) },
      {
        id: 'job-hotkey',
        label: 'Hotkey',
        value: said(humanize(String(config.audio?.hotkey ?? ''))),
        pending: waits('audio.hotkey', 'audio.hotkey_mode'),
      },
      {
        id: 'job-voice',
        label: 'Voice',
        value: said(voices.voices.find((v) => v.id === voice)?.name ?? voice),
        pending: waits('tts.voice', 'tts.speed'),
      },
      { id: 'job-agents', label: 'Agents', value: said(connected > 0 ? `${connected} connected` : 'None yet') },
    ];
  })();

  $: if ($daemon.status) followSaveHistory(savingHistory);

  /// Split from the record, because the two change for different reasons and a
  /// download moves only this one.
  async function readStatus(): Promise<boolean> {
    try {
      const initial = await status();
      wasTranscribing = initial.transcribing === true;
      daemon.update((s) => reduceStatus(s, initial));
      return true;
    } catch (error) {
      const reason = (error as { message?: string })?.message || 'not running';
      daemon.update((s) => ({ ...s, down: reason }));
      startDaemon().catch(() => {});
      return false;
    }
  }

  async function readDaemon() {
    if (!(await readStatus())) return;
    await readLatest();
  }

  async function readVoices(): Promise<boolean> {
    try {
      voices = await listVoices();
      return true;
    } catch {
      voices = { voices: [], current: null };
      return false;
    }
  }

  async function readTheRest() {
    const [gotVoices, gotAgents] = await Promise.all([
      voicesRead || readVoices(),
      agentsRead || readAgents(),
    ]);
    voicesRead = gotVoices;
    agentsRead = gotAgents;
  }

  onMount(async () => {
    // Before the first read, or a stopped daemon misses the push saying it
    // came back. None of these touches the daemon socket.
    await Promise.all([
      listen<Status>('daemon:status', (e) => {
        daemon.update((s) => reduceStatus(s, e.payload));
        if (!$table.loaded) readAll().catch(() => {});
        readTheRest();
      }),
      listen<Partial<Live>>('daemon:state', (e) => {
        daemon.update((s) => reduceLive(s, e.payload));
        if (e.payload.transcribing === false && wasTranscribing) readNewest().catch(() => {});
        if (e.payload.transcribing !== undefined) wasTranscribing = e.payload.transcribing;
      }),
      listen<DownloadProgress>('daemon:downloads', (e) => {
        const progress = e.payload;
        const last = endsTheRun(progress);
        daemon.update((s) => ({ ...s, download: last ? null : progress }));

        // `downloadModels` answers as soon as the daemon takes the task, so a
        // fetch that fails afterwards reaches no caller. This push is the only
        // report of it.
        if (progress.state === 'failed') {
          announce(`${downloadLine(progress)}. Try again when you are back online.`);
        }
        // A file that lands changes what the daemon is blocked on, and no other
        // push says so. The status alone: a download writes no history, so
        // re-reading the record could only return what it already holds.
        if (progress.state !== 'downloading') readStatus().catch(() => {});
        // The voices are files too, and the last one a run fetches is a voice,
        // so their list settles once rather than after every file.
        if (last) {
          voicesRead = false;
          readTheRest().catch(() => {});
        }
      }),
      listen<Down>('daemon:down', (e) =>
        daemon.update((s) => ({ ...s, down: e.payload.reason })),
      ),
      listen('app:reopened', () => readDaemon()),
    ]);
    // The foot needs neither, so it does not queue behind the whole-table read.
    await Promise.all([readDaemon(), readTheRest()]);
  });
</script>

<svelte:window on:keydown={onKeydown} />

<main>
  <Header
    {word}
    {form}
    waiting={live && $waitsOnARestart.size > 0}
    {restart}
    {restarting}
  />

  <div class="body">
    {#if job}
      <Panel name={job} close={() => openJob(null)}>
        {#if job === 'Record'}
          <TheRecord saving={savingHistory} />
        {:else if job === 'Microphone'}
          <MicrophonePanel />
        {:else if job === 'Hotkey'}
          <HotkeyPanel />
        {:else if job === 'Voice'}
          <VoicePanel {voices} />
        {:else if job === 'Agents'}
          <AgentsPanel />
        {/if}
      </Panel>
    {:else}
      {#if finding}
        <Find bind:query matches={shown.length} close={closeFind} />
      {/if}

      {#if blockers.length > 0 || $daemon.download !== null}
        <Blockers
          {blockers}
          {restart}
          busy={restarting}
          download={$daemon.download}
          preset={String(config.stt?.preset ?? 'balanced')}
          megabytes={Number($daemon.status?.download_megabytes ?? 0)}
        />
      {/if}

      {#if !live}
        <Absence
          label="Banshee is not running"
          detail="Dictation and the hotkey do nothing until it starts. What you said before is still here."
          action={starting ? 'Starting' : 'Start Banshee'}
          act={start}
          busy={starting}
        />
      {/if}

      {#if live && !finding && ($table.total > 0 || !savingHistory)}
        <Ledger
          id={RETURNS_TO.ledger}
          total={$table.total}
          saving={savingHistory}
          open={() => openJob('Record', RETURNS_TO.ledger)}
        />
      {/if}

      {#if waiting}
        <Turn
          speaker="agent"
          time={rightNow}
          text="An agent asked a question and is waiting for your answer."
        />
      {:else if pending}
        <Pending mode={pending} time={rightNow} />
      {/if}

      {#if shown.length > 0}
        {#each visible as row, i (row.id)}
          <Turn
            lead={i === 0}
            speaker="user"
            id={String(row.id)}
            time={formatWhen(row.timestamp, now)}
            text={row.text}
          />
        {/each}
        {#if older > 0}
          <div class="older">
            <button class="btn btn-ghost" on:click={() => (limit += PAGE)}>
              {older} older
            </button>
          </div>
        {/if}
      {:else if needle}
        <Absence label="No match" detail={`Nothing said so far contains \u201c${query.trim()}\u201d.`} />
      {:else if !savingHistory}
        <Absence
          label="Nothing is kept"
          detail="Dictation still works and still lands in whatever app has focus. Banshee is simply not writing any of it down."
        />
      {:else if nothingYet && blockers.length === 0}
        <Absence
          label="Nothing said yet"
          detail="Hold the hotkey and speak. What you say lands in whatever app has focus, and shows up here."
          action="What Banshee keeps"
          id={RETURNS_TO.absence}
          act={() => openJob('Record', RETURNS_TO.absence)}
        />
      {/if}
    {/if}
  </div>

  <!-- Foot hands back the label it was given, so the narrowing is sound. -->
  <Foot
    values={footValues}
    active={job}
    open={(name, id) => openJob(job === name ? null : (name as Job), id)}
  />

  <span class="sr" aria-live="polite">{word}{$announcement ? `. ${$announcement}` : ''}</span>
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  /* Exactly one scrolling region, so the foot stays reachable however long the
     day runs. */
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-top: 22px;
  }

  .older {
    margin: 4px var(--gutter) 24px;
  }
</style>
