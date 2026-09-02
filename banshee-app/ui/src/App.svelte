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
  import { announcement, problem, report, spell } from './lib/copy';
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
  import { formatCount } from './lib/history';
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

  // The click destroys the opener, so the way back is its id, never the node.
  const RETURNS_TO = { ledger: 'ledger', absence: 'nothing-yet', agents: 'no-agents' };
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
  // Both are read for their dependency alone: the live state moves the pending
  // caret's time, and adds no row.
  $: now = ($table.rows, $daemon.live, new Date());
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
    // `key`, so the shortcut follows the character the way macOS does: `code`
    // names a physical position, which moves under Dvorak. Lowercased because
    // Caps Lock reports 'F'; the two modifiers were never part of it.
    if (
      (event.metaKey || event.ctrlKey) &&
      !event.shiftKey &&
      !event.altKey &&
      event.key.toLowerCase() === 'f'
    ) {
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
  // The window names no key it has not been told. `audio.hotkey_mode` decides
  // the verb, because "Hold" is a lie when a tap is what starts it.
  $: boundKey = humanize(String(config.audio?.hotkey ?? ''));
  $: holdToTalk = String(config.audio?.hotkey_mode ?? 'hold') !== 'toggle';
  // Said on the home screen only while it is true, so it needs no dismissal to
  // remember: connecting one is what clears it.
  $: noAgentYet = live && agentsRead && connected === 0;

  // Spelled, because a sentence should not open on a digit. Six agents are
  // detectable today, so the list needs no more than this.
  // Name and lead are declared together. "Record" would collide with the RECORDING state word, so
  // the panel is named for what is kept. Every lead reads live state, never the config: the config
  // says only what was asked for.
  $: panels = {
    Microphone: {
      name: 'Microphone',
      lead: $daemon.live.audio_device
        ? `Banshee is listening through the ${$daemon.live.audio_device}.`
        : 'Banshee has no microphone.',
    },
    Hotkey: {
      name: 'Hotkey',
      lead: !boundKey
        ? 'No key is bound, so nothing starts dictation.'
        : holdToTalk
          ? `Hold ${boundKey} to talk.`
          : `Tap ${boundKey} to start, and again to stop.`,
    },
    Voice: {
      name: 'Voice',
      lead: voiceName ? `Banshee speaks as ${voiceName}.` : 'Banshee has no voice yet.',
    },
    Agents: {
      name: 'Agents',
      lead:
        connected === 0
          ? 'No agent can speak to you yet.'
          : `${spell(connected, true)} agent${connected === 1 ? '' : 's'} can speak to you and ask you questions out loud.`,
    },
    Record: {
      name: 'What Banshee keeps',
      lead: !savingHistory
        ? 'Banshee is keeping nothing you say.'
        : $table.total === 0
          ? 'Banshee is keeping what you say, and you have not said anything yet.'
          : `Banshee is keeping ${formatCount($table.total)} things you have said, on this machine.`,
    },
  } as Record<Job, { name: string; lead: string }>;

  $: rows = $table.rows;
  $: needle = query.trim().toLowerCase();
  $: shown = needle ? rows.filter((r) => r.text.toLowerCase().includes(needle)) : rows;
  $: (needle, (limit = PAGE));
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

  // The window restarts the daemon itself rather than naming a terminal command.
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
        if (await readStatus(false)) break;
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

  // Empty when the daemon is stopped. The hotkey is never applied live and the daemon reports no
  // bound key. Pending is the only way the foot stops naming a key nobody listens for.
  // audio.input_device always applies.
  $: voiceName = ((): string => {
    const id = String(config.tts?.voice ?? '');
    return voices.voices.find((v) => v.id === id)?.name ?? id;
  })();

  $: footValues = ((): { id: string; label: Job; value: string; pending?: boolean }[] => {
    const said = (value: string) => (live ? value : '');
    const waits = (...keys: string[]) => live && keys.some((key) => $waitsOnARestart.has(key));
    return [
      {
        id: 'job-microphone',
        label: 'Microphone',
        value: said(microphoneInUse($daemon.live.audio_device)),
      },
      {
        id: 'job-hotkey',
        label: 'Hotkey',
        value: said(humanize(String(config.audio?.hotkey ?? ''))),
        pending: waits('audio.hotkey', 'audio.hotkey_mode'),
      },
      {
        id: 'job-voice',
        label: 'Voice',
        value: said(voiceName),
        pending: waits('tts.voice', 'tts.speed'),
      },
      {
        id: 'job-agents',
        label: 'Agents',
        value: said(connected > 0 ? `${connected} connected` : 'None yet'),
      },
    ];
  })();

  $: if ($daemon.status) followSaveHistory(savingHistory);

  /// andStart is off for the restart poll: launchd is already bringing the daemon back.
  async function readStatus(andStart = true): Promise<boolean> {
    try {
      const initial = await status();
      wasTranscribing = initial.transcribing === true;
      daemon.update((s) => reduceStatus(s, initial));
      return true;
    } catch (error) {
      const reason = (error as { message?: string })?.message || 'not running';
      daemon.update((s) => ({ ...s, down: reason }));
      if (andStart) startDaemon().catch(() => {});
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
          report(`${downloadLine(progress)}. Try again when you are back online.`);
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
      listen<Down>('daemon:down', (e) => daemon.update((s) => ({ ...s, down: e.payload.reason }))),
      listen('app:reopened', () => readDaemon()),
    ]);
    // The foot needs neither, so it does not queue behind the whole-table read.
    await Promise.all([readDaemon(), readTheRest()]);
  });
</script>

<svelte:window on:keydown={onKeydown} />

<main>
  <!-- Invisible until it takes focus. The roving foot collapses four stops into
       one, but the copy controls are the bulk of them and they have to stay
       reachable, so the keyboard needs a way over the record entirely. -->
  <button class="skip" on:click={() => document.getElementById('job-microphone')?.focus()}>
    Skip to the jobs
  </button>

  <Header {word} {form} waiting={live && $waitsOnARestart.size > 0} {restart} {restarting} />

  <div class="body">
    <!-- Outside the panel branch on purpose: a voice that will not play is
         reported from inside a panel, and the reader has to see it there. The
         region is always in the DOM, because one that arrives with its own
         content is not reliably announced. -->
    <div aria-live="polite">
      {#if $problem}
        <Absence label={$problem} action="Dismiss" act={() => problem.set('')} />
      {/if}
    </div>

    {#if job}
      <Panel name={panels[job].name} lead={panels[job].lead} close={() => openJob(null)}>
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
          first={savingHistory && nothingYet}
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

      {#if noAgentYet && !nothingYet && blockers.length === 0}
        <Absence
          label="No agent can speak to you yet"
          detail="A connected agent can ask you questions out loud and hear your answer, so you can leave the screen while it works."
          action="Connect an agent"
          id={RETURNS_TO.agents}
          act={() => openJob('Agents', RETURNS_TO.agents)}
        />
      {/if}

      {#if waiting}
        <!-- The lead while it stands: a person who glances at the window during
             this moment is the one person the window exists for. No copy
             control, because the daemon sends no question to copy. -->
        <Turn
          lead
          speaker="agent"
          time={rightNow}
          text="An agent asked a question and is waiting for your answer."
        >
          {#if boundKey}
            <p class="how">{holdToTalk ? 'Hold' : 'Tap'} {boundKey} to answer.</p>
          {/if}
        </Turn>
      {:else if pending}
        <Pending mode={pending} time={rightNow} />
      {/if}

      {#if shown.length > 0}
        {#each visible as row, i (row.id)}
          <Turn
            lead={i === 0 && !waiting}
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
        <Absence
          label="No match"
          detail={`Nothing said so far contains \u201c${query.trim()}\u201d.`}
        />
      {:else if !savingHistory}
        <Absence
          label="Nothing is kept"
          detail="Dictation still works and still lands in whatever app has focus. Banshee is simply not writing any of it down."
        />
      {:else if nothingYet && blockers.length === 0}
        <Absence
          label="Nothing said yet"
          detail={boundKey
            ? `${holdToTalk ? 'Hold' : 'Tap'} ${boundKey} and speak. What you say lands in whatever app you are using, and shows up here.`
            : 'No key is bound yet, so nothing starts dictation. The Hotkey panel below binds one.'}
          action={panels.Record.name}
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

  /* The readout voice: an instruction the config answers, not a thing anyone
     said. */
  .how {
    margin: 12px 0 0;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--accent);
  }

  .older {
    margin: 4px var(--gutter) 24px;
  }

  .skip {
    position: absolute;
    left: -9999px;
    z-index: 1;
    background: var(--ink);
    color: var(--ground);
    border: 0;
    border-radius: 0;
    padding: 8px 14px;
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    cursor: pointer;
  }

  .skip:focus {
    left: var(--gutter);
    top: 8px;
  }
</style>
