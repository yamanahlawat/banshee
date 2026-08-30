<script lang="ts">
  import { onMount } from 'svelte';
  import {
    daemon,
    lampForm,
    microphoneInUse,
    reduceLive,
    reduceStatus,
    stateWord,
    type Live,
    type Status,
  } from './lib/daemon';
  import { announcement } from './lib/copy';
  import { followSaveHistory, readAll, readNewest, table } from './lib/history';
  import { agents, refresh as readAgents } from './lib/agents';
  import {
    listen,
    listVoices,
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
      job = null;
      finding = true;
      return;
    }
    if (event.key !== 'Escape') return;
    if (job !== null) job = null;
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
  $: savingHistory = config.daemon?.save_history !== false;

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
  $: footValues = ((): { label: Job; value: string }[] => {
    const voice = String(config.tts?.voice ?? '');
    const said = (value: string) => (live ? value : '');
    return [
      { label: 'Microphone', value: said(microphoneInUse($daemon.live.audio_device)) },
      { label: 'Hotkey', value: said(humanize(String(config.audio?.hotkey ?? ''))) },
      { label: 'Voice', value: said(voices.voices.find((v) => v.id === voice)?.name ?? voice) },
      { label: 'Agents', value: said(connected > 0 ? `${connected} connected` : 'None yet') },
    ];
  })();

  $: if ($daemon.status) followSaveHistory(savingHistory);

  async function readDaemon() {
    try {
      const initial = await status();
      wasTranscribing = initial.transcribing === true;
      daemon.update((s) => reduceStatus(s, initial));
    } catch (error) {
      const reason = (error as { message?: string })?.message || 'not running';
      daemon.update((s) => ({ ...s, down: reason }));
      startDaemon().catch(() => {});
      return;
    }
    await ($table.loaded ? readNewest() : readAll()).catch(() => {});
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
      listen<DownloadProgress>('daemon:downloads', (e) =>
        daemon.update((s) => ({ ...s, downloading: e.payload.state === 'downloading' })),
      ),
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
  <Header {word} {form} />

  <div class="body">
    {#if job}
      <Panel name={job} close={() => (job = null)}>
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

      {#if blockers.length > 0}
        <Blockers {blockers} downloading={$daemon.downloading} />
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

      {#if live && !finding}
        <Ledger
          total={$table.total}
          saving={savingHistory}
          open={() => (job = 'Record')}
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
        />
      {/if}
    {/if}
  </div>

  <!-- Foot hands back the label it was given, so the narrowing is sound. -->
  <Foot
    values={footValues}
    active={job}
    open={(name) => (job = job === name ? null : (name as Job))}
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
