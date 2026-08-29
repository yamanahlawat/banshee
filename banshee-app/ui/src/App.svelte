<script lang="ts">
  import { onMount } from 'svelte';
  import { daemon, deviceLabel, reduceLive, reduceStatus, setupBlocked, stateWord, SYSTEM_DEVICE, type Live, type Status } from './lib/daemon';
  import { announcement } from './lib/copy';
  import { followSaveHistory, moreCount, readAll, readNewest, table, today } from './lib/history';
  import { listen, listVoices, startDaemon, status, type Down, type DownloadProgress, type Voices } from './lib/tauri';
  import { agents, refresh as readAgents } from './lib/agents';
  import TitleBar from './bands/TitleBar.svelte';
  import Scale from './bands/Scale.svelte';
  import SetupFixes from './bands/SetupFixes.svelte';
  import Pad from './bands/Pad.svelte';
  import Earlier from './bands/Earlier.svelte';
  import Strip from './bands/Strip.svelte';
  import Job from './bands/Job.svelte';
  import Microphone from './jobs/Microphone.svelte';
  import HotkeyJob from './jobs/Hotkey.svelte';
  import Voice from './jobs/Voice.svelte';
  import Agents from './jobs/Agents.svelte';
  import History from './jobs/History.svelte';
  import { BANDED, open } from './lib/jobs';
  import { humanize } from './lib/hotkey';

  let wasTranscribing = false;
  let voices: Voices = { voices: [], current: null };
  let voicesRead = false;
  let agentsRead = false;

  $: landing = $daemon.live.recording ? '' : null;
  $: word = stateWord($daemon);
  $: config = ($daemon.status?.config ?? {}) as Record<string, Record<string, unknown>>;
  // The daemon names a voice by id, and the strip says what a person calls it.
  $: voiceName = (id: string) => voices.voices.find((v) => v.id === id)?.name ?? id;
  $: connectedAgents = $agents.filter((a) => a.presence === 'connected').length;
  // The strip has room for the device alone, so it drops the word the panel
  // spells out beside it.
  $: inputDevice = String(config.audio?.input_device ?? '');
  $: microphone = String($daemon.live.audio_device ?? '') ||
    (inputDevice === SYSTEM_DEVICE ? deviceLabel(null) : inputDevice);
  // A stopped daemon knows none of these. Setup is the row that has most to
  // say without one, so it keeps its word.
  $: live = stateWord($daemon) !== 'Not running';
  $: whenLive = (value: string) => (live ? value : '');
  $: stripValues = {
    Microphone: whenLive(microphone),
    Hotkey: whenLive(humanize(String(config.audio?.hotkey ?? ''))),
    Voice: whenLive(voiceName(String(config.tts?.voice ?? ''))),
    Agents: whenLive(connectedAgents > 0 ? `${connectedAgents} connected` : 'None yet'),
    Setup: setupBlocked($daemon) ? 'To fix' : 'All clear',
    'More settings': whenLive('History'),
  };
  // A dead daemon fails the history read, so waiting for it would leave the
  // pad blank in the one state that most needs its fix.
  $: setup = setupBlocked($daemon) || ($table.loaded && $table.total === 0);
  // The pad holds the newest row, so the band starts below it. The fixes
  // hold no row, so when they stand in its place the band starts at the top.
  $: padHolds = setup ? 0 : 1;
  $: latest = $table.rows[0] ?? null;
  // The band holds the whole day, so the count below it is what History
  // alone can reach.
  $: earlierRows = today($table.rows, new Date(), padHolds);
  $: beyondTheBand = moreCount($table.total - padHolds, earlierRows.length);
  // The daemon refuses the read outright while this is off, so the table
  // follows the switch rather than waiting for the panel to be opened again.
  $: if ($daemon.status) followSaveHistory(config.daemon?.save_history !== false);

  // A stopped daemon fails both reads. The bridge pushes a status once it
  // reaches the daemon, so the window reports Not running and waits.
  async function readDaemon() {
    try {
      const initial = await status();
      // A window opened mid-dictation has to know a fall is coming.
      wasTranscribing = initial.transcribing === true;
      daemon.update((s) => reduceStatus(s, initial));
    } catch (error) {
      // The command's own sentence names the cause; "not running" is only
      // the fallback when it carries none.
      const reason = (error as { message?: string })?.message || 'not running';
      daemon.update((s) => ({ ...s, down: reason }));
      // Opening Banshee starts Banshee, and this is the first moment the
      // window knows it must. Starting one that already runs would replace it.
      startDaemon().catch(() => {});
      return;
    }
    // The daemon refuses the history read outright while `save_history` is
    // off, and that must not report a running daemon as down.
    await ($table.loaded ? readNewest() : readAll()).catch(() => {});
  }

  // The strip only needs these to name a voice and count the agents, so a
  // failure must not reach the status and the history beside it. Each answers
  // whether the read landed, and the caller holds that.
  async function readVoices(): Promise<boolean> {
    try {
      voices = await listVoices();
      return true;
    } catch {
      voices = { voices: [], current: null };
      return false;
    }
  }

  // The window opens before the daemon answers, so a read that failed at mount
  // is tried again on the push that says it can be answered. Neither read needs
  // the other, and each holds its own fallback.
  async function readTheRest() {
    const [gotVoices, gotAgents] = await Promise.all([
      voicesRead || readVoices(),
      agentsRead || readAgents(),
    ]);
    voicesRead = gotVoices;
    agentsRead = gotAgents;
  }

  onMount(async () => {
    // The listeners come first, or a stopped daemon leaves the window deaf
    // to the push that says it came back.
    await listen<Status>('daemon:status', (e) => {
      daemon.update((s) => reduceStatus(s, e.payload));
      if (!$table.loaded) readAll().catch(() => {});
      readTheRest();
    });
    await listen<Partial<Live>>('daemon:state', (e) => {
      daemon.update((s) => reduceLive(s, e.payload));
      if (e.payload.transcribing === false && wasTranscribing) readNewest().catch(() => {});
      if (e.payload.transcribing !== undefined) wasTranscribing = e.payload.transcribing;
    });
    await listen<DownloadProgress>('daemon:downloads', (e) => daemon.update((s) => ({ ...s, downloading: e.payload.state === 'downloading' })));
    await listen<Down>('daemon:down', (e) => daemon.update((s) => ({ ...s, down: e.payload.reason })));
    // A second open reaches this window rather than a new one, so the read
    // that starts the daemon runs again here instead of at a mount.
    await listen('app:reopened', () => readDaemon());
    await readDaemon();
    await readTheRest();
  });
</script>

<main style="display: flex; flex-direction: column; height: 100%;">
  <TitleBar />
  <Scale compact={word === 'Recording'} />
  <!-- Only this region scrolls, so the strip stays reachable however long
       today's history runs. -->
  <div style="flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column;">
    {#if $open === 'More settings'}
      <!-- History replaces both the pad and the earlier list while it is open. -->
      <History />
    {:else}
      {#if setup}
        <SetupFixes />
      {:else}
        <Pad {latest} {landing} />
      {/if}
      <!-- A job opens above the strip and stands in the earlier list's place. -->
      {#if $open !== null && BANDED.includes($open)}
        <Job name={$open}>
          {#if $open === 'Microphone'}<Microphone />{:else if $open === 'Hotkey'}<HotkeyJob />{:else if $open === 'Voice'}<Voice {voices} />{:else if $open === 'Agents'}<Agents />{/if}
        </Job>
      {:else}
        <Earlier rows={earlierRows} more={beyondTheBand} history={$table.loaded ? ($table.total > 0 ? 'some' : 'empty') : 'unread'} />
      {/if}
    {/if}
  </div>
  <Strip values={stripValues} />
  <!-- The region holds only what must be spoken. On `main` it would announce
       every row the list redraws and every word the title bar swaps. -->
  <span class="sr" aria-live="polite">{word}{$announcement ? `. ${$announcement}` : ''}</span>
</main>
