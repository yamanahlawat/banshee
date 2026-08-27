<script lang="ts">
  import { onMount } from 'svelte';
  import { daemon, reduceLive, reduceStatus, stateWord, type Live, type Status } from './lib/daemon';
  import { announcement } from './lib/copy';
  import { countNewer, newestFirst, nextLimit, PAGE, today } from './lib/history';
  import { history, listen, status, type Down, type DownloadProgress, type HistoryRow } from './lib/tauri';
  import TitleBar from './bands/TitleBar.svelte';
  import Pad from './bands/Pad.svelte';
  import Earlier from './bands/Earlier.svelte';

  let rows: HistoryRow[] = [];
  let total = 0;
  let wasTranscribing = false;
  let loaded = false;
  let loading: Promise<void> | null = null;

  // One unlimited read on open is the only source for the total.
  async function readWholeTable() {
    const all = await history();
    total = all.length;
    rows = newestFirst(all).slice(0, PAGE);
    loaded = true;
  }

  // The first read and the bridge's own status push both reach this while
  // neither has finished, so they share one read of the table.
  function loadAll(): Promise<void> {
    loading ??= readWholeTable().finally(() => {
      loading = null;
    });
    return loading;
  }

  // The daemon stores the row before it reports transcribing finished, so
  // that fall is the first moment a refetch can see the new dictation.
  async function refresh() {
    const page = newestFirst(await history(nextLimit(PAGE)));
    const added = countNewer(page, rows[0]?.id ?? null);
    if (added === null) {
      await loadAll();
      return;
    }
    total += added;
    rows = page.slice(0, PAGE);
  }

  // The pad already shows the newest row, so the earlier list starts below it.
  $: latest = rows[0] ?? null;
  $: earlierRows = today(rows.slice(1), new Date());
  $: earlierTotal = Math.max(total - 1, 0);
  $: landing = $daemon.live.recording ? '' : null;
  $: word = stateWord($daemon);

  // A stopped daemon fails both reads. The bridge pushes a status once it
  // reaches the daemon, so the window reports Not running and waits.
  async function readDaemon() {
    try {
      const initial = await status();
      // A window opened mid-dictation has to know a fall is coming.
      wasTranscribing = initial.transcribing === true;
      daemon.update((s) => reduceStatus(s, initial));
      await loadAll();
    } catch (error) {
      // The command's own sentence names the cause; "not running" is only
      // the fallback when it carries none.
      const reason = (error as { message?: string })?.message || 'not running';
      daemon.update((s) => ({ ...s, down: reason }));
    }
  }

  onMount(async () => {
    // The listeners come first, or a stopped daemon leaves the window deaf
    // to the push that says it came back.
    await listen<Status>('daemon:status', (e) => {
      daemon.update((s) => reduceStatus(s, e.payload));
      if (!loaded) loadAll();
    });
    await listen<Partial<Live>>('daemon:state', (e) => {
      daemon.update((s) => reduceLive(s, e.payload));
      if (e.payload.transcribing === false && wasTranscribing) refresh();
      if (e.payload.transcribing !== undefined) wasTranscribing = e.payload.transcribing;
    });
    await listen<DownloadProgress>('daemon:downloads', (e) => daemon.update((s) => ({ ...s, downloading: e.payload.state === 'downloading' })));
    await listen<Down>('daemon:down', (e) => daemon.update((s) => ({ ...s, down: e.payload.reason })));
    await readDaemon();
  });
</script>

<main style="display: flex; flex-direction: column;">
  <TitleBar />
  <Pad {latest} {landing} agent={null} />
  <Earlier rows={earlierRows} total={earlierTotal} />
  <!-- The region holds only what must be spoken. On `main` it would announce
       every row the list redraws and every word the title bar swaps. -->
  <span class="sr" aria-live="polite">{word}{$announcement ? `. ${$announcement}` : ''}</span>
</main>
