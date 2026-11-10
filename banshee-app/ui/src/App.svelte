<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { land } from './lib/focus';
  import {
    daemon,
    downloadLine,
    endsTheRun,
    hotkeyListens as listensForHotkey,
    isDown,
    listeningFacts,
    applyPush,
    applyPushedStatus,
    refreshStatus,
    readinessIsStale,
    speechFacts,
    waiting as waitingKeys,
    stateWord,
    type Live,
    type Status,
  } from './lib/daemon';
  import {
    announcement,
    announcer,
    listeningLead,
    problem,
    report,
    reports,
    speechLead,
    DICTATION_FAILED,
    DICTATION_FAILURE,
    failureSays,
    audioGoesTo,
    SPEECH_FAILED,
    SPEECH_FAILURE,
    failureReads,
    spell,
    feedbackLead,
  } from './lib/copy';
  import {
    followSaveHistory,
    formatCount,
    newestUnanswered,
    readAll,
    readLatest,
    readNewest,
    table,
    view,
  } from './lib/history';
  import { agents, refresh as readAgents } from './lib/agents';
  import { loadTable } from './lib/settings';
  import { forgetTheAsk } from './lib/downloads';
  import {
    launchMissed,
    listen,
    listVoices,
    restartDaemon,
    startDaemon,
    status,
    type Down,
    type DownloadProgress,
    type Reopened,
    type Voices,
  } from './lib/tauri';
  import { humanize } from './lib/hotkey';
  import { claimKeys, keysClaimed, onMac } from './lib/keys';
  import Sidebar from './bands/Sidebar.svelte';
  import Page from './bands/Page.svelte';
  import { PAGE_HEADING, SIDEBAR } from './bands/page';
  import { listAndDetail, sidebarShown } from './lib/layout';
  import { waitingOn, type Page as PageName } from './lib/nav';
  import Blockers from './bands/Blockers.svelte';
  import Find from './bands/Find.svelte';
  import HostLine from './bands/HostLine.svelte';
  import Absence from './turns/Absence.svelte';
  import HistoryPage from './jobs/HistoryPage.svelte';
  import ListeningPage from './jobs/ListeningPage.svelte';
  import HotkeyPage from './jobs/HotkeyPage.svelte';
  import SpeakingPage from './jobs/SpeakingPage.svelte';
  import AgentsPage from './jobs/AgentsPage.svelte';
  import FeedbackPage from './jobs/FeedbackPage.svelte';
  import SavedHistoryPage from './jobs/SavedHistoryPage.svelte';

  let seenChanges: number | null = null;
  let seenSettings: number | null = null;
  let wasLoadingModel = false;
  let voices: Voices = { voices: [], current: null };
  let voicesRead = false;
  let agentsRead = false;
  // Nothing remembers the page, so every launch opens on History.
  let page: PageName = 'History';
  // The platform does not change under this window.
  const mac = onMac();

  // `updateCallbackDone` settles before the crossfade finishes.
  type ViewTransition = { updateCallbackDone: Promise<void> };
  // Awaited, because the platform runs the callback in a later task once it has
  // the old snapshot. Returning before then leaves the focus move below looking
  // for a page that has not rendered.
  async function swap(run: () => void): Promise<void> {
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
    const start = (
      document as Document & { startViewTransition?: (run: () => void) => ViewTransition }
    ).startViewTransition;
    if (reduce || !start) {
      run();
      return;
    }
    await start.call(document, run).updateCallbackDone.catch(() => {});
  }

  // One scroller serves every page, so a page arrives at its own top.
  let body: HTMLElement;
  async function show(next: PageName) {
    if (next === page) return;
    await swap(() => {
      page = next;
      if (body) body.scrollTop = 0;
    });
  }

  /// `at` names an element on the page to land on in place of its heading.
  async function go(next: PageName, at = '') {
    await show(next);
    await land(() => document.getElementById(at || PAGE_HEADING));
  }

  let drawer: HTMLDialogElement;
  let unfold: HTMLButtonElement | null = null;
  // Held while the drawer is open, so no key reaches the page behind it.
  let drawerKeys: (() => void) | null = null;

  function openDrawer() {
    drawer.showModal();
    drawerKeys = claimKeys();
  }

  function closedDrawer() {
    drawerKeys?.();
    drawerKeys = null;
    unfold?.focus();
  }

  async function chooseFromDrawer(next: PageName) {
    drawer.close();
    await show(next);
  }

  // Runs before the DOM swaps one sidebar for the other, so focus is read
  // while the sidebar it sits in still exists. A dialog removed while open
  // fires no close event.
  function refold(shown: boolean) {
    if (shown && drawerKeys) {
      drawerKeys();
      drawerKeys = null;
      land(() =>
        document.getElementById(SIDEBAR)?.querySelector<HTMLElement>('[aria-current="page"]'),
      );
    } else if (!shown && document.getElementById(SIDEBAR)?.contains(document.activeElement)) {
      land(() => unfold);
    }
  }
  $: refold($sidebarShown);

  let search: Find | undefined;
  // Moved by every reopen, which draws History afresh, following the newest row.
  let reopened = 0;
  let history: HistoryPage | undefined;
  let reading = false;
  let back: HTMLButtonElement | null = null;

  // Runs before the DOM drops Back, so focus is read while Back still holds it.
  function unback(wide: boolean) {
    if (wide && back && document.activeElement === back) tick().then(() => history?.focusDetail());
  }
  $: unback($listAndDetail);

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
      show('History').then(async () => {
        await tick();
        await search?.open();
      });
    }
  }

  $: word = stateWord($daemon);
  $: config = ($daemon.status?.config ?? {}) as Record<string, Record<string, unknown>>;
  $: live = !isDown($daemon);
  $: connected = $agents.filter((a) => a.presence === 'connected').length;
  // Only once the daemon has answered, so a failed read never claims none.
  $: agentless = live && agentsRead && connected === 0;
  // Wayland grants no global grab, so the daemon binds nothing and says so.
  $: hotkeyListens = listensForHotkey($daemon);
  // The window names no key it has not been told. `audio.hotkey_mode` decides
  // the verb, because "Hold" is a lie when a tap is what starts it.
  $: boundKey = humanize(String(config.audio?.hotkey ?? ''));
  $: holdToTalk = String(config.audio?.hotkey_mode ?? 'hold') !== 'toggle';

  // Dictation is the way in, so it takes the one slot when both sides failed.
  // The daemon clears each on the next one that works, so neither is dismissed
  // here.
  $: failed = !live
    ? null
    : $daemon.live.last_error
      ? {
          page: 'Listening' as PageName,
          at: DICTATION_FAILURE,
          said: failureSays(DICTATION_FAILED, $daemon.live.last_error),
          reads: failureReads(DICTATION_FAILED, $daemon.live.last_error),
        }
      : $daemon.live.last_speech_error
        ? {
            page: 'Speaking' as PageName,
            at: SPEECH_FAILURE,
            said: failureSays(SPEECH_FAILED, $daemon.live.last_speech_error),
            reads: failureReads(SPEECH_FAILED, $daemon.live.last_speech_error),
          }
        : null;

  // The window says it, not the page: a page is mounted only while it shows,
  // and a failure arrives with no control moving.
  const sawFailure = announcer<string | null>();
  $: said = failed?.said ?? null;
  $: sawFailure(said, said ?? '');

  // The daemon says which listener and which speaker are in force; the config
  // says only which ones were asked for. The heading, the page's note and the
  // sidebar foot all read one record of each side, so no two of them can state
  // a different truth.
  $: listening = listeningFacts($daemon);
  $: host = audioGoesTo(listening);
  $: speech = speechFacts($daemon, voices.voices);
  // The opening line names a host only while text reaches it.
  $: textLeavesTo = speech.started ? speech.host : null;

  // Every lead reads live state, never the config: the config says only what
  // was asked for. History has no statement, so its title heads it.
  $: leads = {
    History: null,
    // Spelled, because a sentence should not open on a digit. Six agents are
    // detectable today, so the list needs no more than this.
    Agents:
      connected === 0
        ? 'No agent can speak to you yet.'
        : `${spell(connected, true)} agent${connected === 1 ? '' : 's'} can speak to you and ask you questions out loud.`,
    Listening: listeningLead(listening),
    Speaking: speechLead(speech),
    Hotkey: !hotkeyListens
      ? 'Your compositor holds this binding. Banshee binds no key here.'
      : !boundKey
        ? 'No key is bound, so nothing starts dictation.'
        : holdToTalk
          ? `Hold ${boundKey} to talk.`
          : `Tap ${boundKey} to start, and again to stop.`,
    Feedback: feedbackLead($daemon.live.feedback, live, mac),
    'Saved history': !savingHistory
      ? 'Banshee is keeping nothing new you say.'
      : $table.total === 0
        ? 'Banshee is keeping what you say, and you have not said anything yet.'
        : `Banshee is keeping ${formatCount($table.total)} row${$table.total === 1 ? '' : 's'}, on this machine.`,
  } satisfies Record<PageName, string | null>;

  // The daemon sets `armed` from `ask_user` alone, and sends no question with
  // it, so History can say an agent is waiting and nothing further.
  $: waiting = $daemon.live.armed;
  $: nothingYet = $table.loaded && $table.rows.length === 0;
  $: blockers = $daemon.status?.blockers ?? [];
  $: downloading = $daemon.download !== null;
  $: blocking = blockers.length > 0 || downloading;
  // Each says why History may hold nothing new, so it stands above the list.
  $: alerting = blocking || !live || failed !== null;
  // The daemon answers this, and the config only says what was asked for. A
  // history file that will not open leaves the two disagreeing.
  $: savingHistory = ($daemon.status?.history_enabled ?? config.daemon?.save_history) !== false;

  // The window restarts the daemon itself rather than naming a terminal command.
  let restarting = false;
  // launchd.plist(5): ThrottleInterval, the least time between two launches of a
  // job, defaults to 10 seconds.
  const LAUNCHD_THROTTLE_MS = 10_000;
  const POLL_MS = 250;
  async function restart() {
    restarting = true;
    try {
      await restartDaemon();
      // The restart answers once the daemon has the request, not when the new
      // process listens, and launchd may hold the relaunch for its throttle.
      // Reading straight away finds a dead socket and tells the user Banshee
      // has stopped, which is the opposite of what just happened.
      for (let left = LAUNCHD_THROTTLE_MS / POLL_MS; left > 0; left--) {
        if (await readStatus(false)) break;
        await new Promise((wake) => setTimeout(wake, POLL_MS));
      }
      await readLatest();
    } catch {
      // The blocker still stands, so its box already says it.
    }
    restarting = false;
  }

  let starting = false;
  /// What `banshee start` said when it refused, such as a config it will not read.
  let startRefusal = '';
  function startRefused(error: unknown) {
    startRefusal = (error as { message?: string })?.message || '';
  }

  async function start() {
    starting = true;
    try {
      await startDaemon();
      await readDaemon();
    } catch (error) {
      startRefused(error);
    }
    starting = false;
  }

  // A stopped daemon has no key to apply and no failure in force.
  $: waitingPages = live ? waitingOn($waitingKeys) : {};
  $: warned = [
    ...(live && $daemon.live.last_error ? ['Listening' as const] : []),
    ...(live && $daemon.live.last_speech_error ? ['Speaking' as const] : []),
  ];

  $: if ($daemon.status) followSaveHistory(savingHistory);

  // A grant, a hand-edited config.toml and a model file added outside Banshee
  // reach no push, and returning is the one moment that catches them all.
  // `andStart` stays off: a return is not a request to start a stopped daemon.
  let rereading = false;
  async function onReturn() {
    if (document.visibilityState === 'hidden' || rereading) return;
    rereading = true;
    try {
      await readStatus(false);
    } finally {
      rereading = false;
    }
  }

  function advance(seen: number | null, now: number | undefined): [number | null, boolean] {
    if (now === undefined) return [seen, false];
    return [now, seen !== null && now !== seen];
  }

  // Every kept row and every clear moves the count, whatever the activity was.
  function noteChanges(now: number | undefined) {
    let moved: boolean;
    [seenChanges, moved] = advance(seenChanges, now);
    if (moved) readNewest().catch(() => {});
  }

  // A read only records the count; a push that moves it is what asks for a read.
  function noteSettingsChanges(now: number | undefined): boolean {
    let moved: boolean;
    [seenSettings, moved] = advance(seenSettings, now);
    return moved;
  }

  /// andStart is off for the restart poll: launchd is already bringing the daemon back.
  async function readStatus(andStart = true): Promise<boolean> {
    try {
      await refreshStatus(status);
      startRefusal = '';
      noteChanges($daemon.live.history_changes);
      noteSettingsChanges($daemon.live.settings_changes);
      wasLoadingModel = $daemon.live.loading_model;
      return true;
    } catch (error) {
      const reason = (error as { message?: string })?.message || 'not running';
      daemon.update((s) => ({ ...s, down: reason }));
      if (andStart) startDaemon().catch(startRefused);
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

  function openAt(missed: boolean) {
    const chosen = missed ? (newestUnanswered($table.rows)?.id ?? null) : null;
    view.set({ query: '', chosen });
    if (chosen !== null) history?.reveal(chosen);
  }

  onMount(() => {
    // Focus alone misses a window unhidden without the pointer landing in it.
    document.addEventListener('visibilitychange', onReturn);
    return () => document.removeEventListener('visibilitychange', onReturn);
  });

  onMount(async () => {
    // Before the first read, or a stopped daemon misses the push saying it
    // came back. None of these touches the daemon socket.
    await Promise.all([
      listen<Status>('daemon:status', (e) => {
        applyPushedStatus(e.payload);
        if (!$table.loaded) readAll().catch(() => {});
        readTheRest();
      }),
      listen<Partial<Live> & Pick<Status, 'pipeline'>>('daemon:state', (e) => {
        if (readinessIsStale($daemon, e.payload)) readStatus(false);
        applyPush(e.payload);
        noteChanges(e.payload.history_changes);
        if (noteSettingsChanges(e.payload.settings_changes)) readStatus(false);
        // `stt_model` and `stt_translates` come only with a full status, and the
        // read a write does happens before the file lands, so this is the only
        // word that the model in force moved. Held rather than read off the
        // store, which `applyPush` above has already advanced.
        if (e.payload.loading_model === false && wasLoadingModel) readStatus(false);
        if (e.payload.loading_model !== undefined) wasLoadingModel = e.payload.loading_model;
      }),
      listen<DownloadProgress>('daemon:downloads', (e) => {
        const progress = e.payload;
        const last = endsTheRun(progress);
        forgetTheAsk();
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
      // Opening the window moves no focus: the user is always inside another
      // application.
      listen<Reopened>('app:reopened', async (e) => {
        openAt(false);
        reopened += 1;
        show('History');
        await readDaemon();
        if (e.payload.missed) openAt(true);
      }),
    ]);
    const missed = launchMissed();
    // The sidebar needs neither, so it does not queue behind the whole-table read.
    await Promise.all([loadTable(), readDaemon(), readTheRest()]);
    if (await missed) openAt(true);
  });
</script>

<svelte:window on:keydown={onKeydown} on:focus={onReturn} />

<div class="window" class:folded={!$sidebarShown}>
  <!-- Invisible until it takes focus. Seven sidebar stops come first, so the
       keyboard needs a way past them to the page. -->
  <button class="btn skip" on:click={() => document.getElementById(PAGE_HEADING)?.focus()}>
    Skip to the page
  </button>

  {#if $sidebarShown}
    <Sidebar {page} {go} state={word} {host} pending={waitingPages} {warned} />
  {:else}
    <dialog class="drawer" aria-label="Sidebar" bind:this={drawer} on:close={closedDrawer}>
      <Sidebar {page} go={chooseFromDrawer} state={word} {host} pending={waitingPages} {warned} />
    </dialog>
  {/if}

  <main>
    <h1 class="sr">Banshee</h1>
    <Page name={page} lead={leads[page]} regions={page === 'History'} bind:scroller={body}>
      <svelte:fragment slot="start">
        {#if !$sidebarShown}
          <button class="btn btn-ghost unfold" bind:this={unfold} on:click={openDrawer}>
            <span class="sr">Show sidebar</span>
            <svg class="icon" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
              <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
              <path d="M6 2.5 V13.5" />
            </svg>
          </button>
        {/if}
        {#if page === 'History' && reading}
          <button class="btn btn-ghost back" bind:this={back} on:click={() => history?.back()}>
            <svg class="icon" viewBox="0 0 12 12" aria-hidden="true" focusable="false">
              <path d="M7.5 2.5 L4 6 L7.5 9.5" />
            </svg>
            <span class="sr">Back to</span> History
          </button>
        {/if}
      </svelte:fragment>
      <svelte:fragment slot="end">
        {#if page === 'History' && $table.rows.length > 0}
          <Find bind:this={search} />
        {/if}
      </svelte:fragment>
      <!-- Above every page on purpose: a voice that will not play is reported
           from inside a page, and the reader has to see it there. The region is
           always in the DOM, because one that arrives with its own content is
           not reliably announced. -->
      <svelte:fragment slot="notice">
        <!-- History draws it at the top of its own notices. -->
        {#if !$sidebarShown && host !== null && page !== 'History'}
          <HostLine {host} inset />
        {/if}
        <div aria-live="polite">
          {#key $reports}
            {#if $problem}
              <Absence label={$problem} action="Dismiss" act={() => problem.set('')} />
            {/if}
          {/key}
        </div>
      </svelte:fragment>

      {#if page === 'Agents'}
        <AgentsPage />
      {:else if page === 'Listening'}
        <ListeningPage />
      {:else if page === 'Speaking'}
        <SpeakingPage {voices} />
      {:else if page === 'Hotkey'}
        <HotkeyPage />
      {:else if page === 'Feedback'}
        <FeedbackPage />
      {:else if page === 'Saved history'}
        <SavedHistoryPage saving={savingHistory} />
      {:else}
        {#key reopened}
          <HistoryPage
            bind:this={history}
            bind:reading
            saving={savingHistory}
            {alerting}
            {downloading}
            {agentless}
            {waiting}
            {host}
            compositor={!hotkeyListens}
            hotkey={boundKey}
            verb={holdToTalk ? 'Hold' : 'Tap'}
            {go}
          >
            <svelte:fragment slot="alerts">
              {#if blocking}
                <Blockers
                  {blockers}
                  {restart}
                  busy={restarting}
                  download={$daemon.download}
                  models={$daemon.status?.stt_models ?? []}
                  chosen={String(config.stt?.model ?? '')}
                  megabytes={Number($daemon.status?.download_megabytes ?? 0)}
                  first={savingHistory && nothingYet}
                  remoteHost={listening.stoppedBy === null ? listening.host : null}
                  speechHost={textLeavesTo}
                />
              {/if}

              {#if !live}
                <Absence
                  label="Banshee is not running"
                  detail={startRefusal ||
                    'Dictation and the hotkey do nothing until it starts. What you said before is still here.'}
                  action={starting ? 'Starting' : 'Start Banshee'}
                  act={start}
                  busy={starting}
                />
              {/if}

              {#if failed}
                <!-- The sidebar marks the page in two words; this states it where the
                     reader looks, with the fix in a button like a blocker's. -->
                <div class="failure">
                  <p class="note">{failed.reads}</p>
                  <button
                    class="btn btn-ghost"
                    on:click={() => failed && go(failed.page, failed.at)}
                  >
                    Open {failed.page}
                  </button>
                </div>
              {/if}
            </svelte:fragment>
          </HistoryPage>
        {/key}
      {/if}
    </Page>

    <span class="sr" aria-live="polite">{word}{$announcement ? `. ${$announcement}` : ''}</span>
  </main>
</div>

<style>
  .window {
    display: grid;
    grid-template-columns: 208px minmax(0, 1fr);
    height: 100%;
    overflow: hidden;
  }

  .window.folded {
    grid-template-columns: minmax(0, 1fr);
  }

  .drawer {
    width: 208px;
    max-width: none;
    height: 100%;
    max-height: none;
    margin: 0;
    padding: 0;
    border: 0;
    background: transparent;
  }

  .drawer[open] {
    display: grid;
  }

  /* `::backdrop` inherits no custom property before Safari 17.4, so each
     appearance writes its tint out. */
  .drawer::backdrop {
    background: rgba(8, 12, 15, 0.14);
  }

  @media (prefers-color-scheme: dark) {
    .drawer::backdrop {
      background: rgba(0, 0, 0, 0.5);
    }
  }

  .unfold {
    display: flex;
    flex: none;
    padding: 7px;
  }

  .unfold svg {
    width: 16px;
    height: 16px;
  }

  .back {
    display: flex;
    flex: none;
    align-items: center;
    gap: 4px;
  }

  .back svg {
    width: 12px;
    height: 12px;
  }

  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  /* The swap hurries: a reader catching up never waits on it. Global, because
     the snapshot tree lives outside any component's scope. */
  :global(::view-transition-old(root)),
  :global(::view-transition-new(root)) {
    animation-duration: 180ms;
  }

  /* Ink, not the accent: this is prose, and the accent does not emphasise
     prose. The sidebar's mark carries the alarm in the accent, and two lines of
     it here would say the same thing a second time, louder. */
  .failure {
    max-width: 520px;
    margin: 0 var(--gutter) 16px;
  }

  .failure p {
    margin: 0 0 12px;
    color: var(--ink);
  }

  .skip {
    position: absolute;
    left: -9999px;
    z-index: 1;
  }

  .skip:focus {
    left: var(--gutter);
    top: 8px;
  }
</style>
