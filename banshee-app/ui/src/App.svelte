<script lang="ts">
  import { onMount } from 'svelte';
  import { daemon, reduceLive, reduceStatus, type Live, type Status } from './lib/daemon';
  import { announcement } from './lib/copy';
  import { newestFirst, nextLimit } from './lib/history';
  import { history, listen, status, type Down, type DownloadProgress, type HistoryRow } from './lib/tauri';
  import TitleBar from './bands/TitleBar.svelte';
  import Pad from './bands/Pad.svelte';
  import Earlier from './bands/Earlier.svelte';

  let rows: HistoryRow[] = [];
  let total = 0;
  let wasRecording = false;

  async function loadAll() {
    const all = await history();
    total = all.length;
    rows = newestFirst(all);
  }

  // A new dictation only shows up once recording stops, so a limited
  // refetch there catches it without reading the whole table again.
  async function refresh() {
    const page = await history(nextLimit(rows.length));
    rows = newestFirst(page);
  }

  // The pad already shows the newest row, so the earlier list starts below it.
  $: latest = rows[0] ?? null;
  $: earlierRows = rows.slice(1);
  $: earlierTotal = Math.max(total - 1, 0);
  $: landing = $daemon.live.recording ? '' : null;

  onMount(async () => {
    const initial = await status();
    daemon.update((s) => reduceStatus(s, initial));
    await loadAll();
    await listen<Status>('daemon:status', (e) => daemon.update((s) => reduceStatus(s, e.payload)));
    await listen<Partial<Live>>('daemon:state', (e) => {
      daemon.update((s) => reduceLive(s, e.payload));
      if (e.payload.recording === false && wasRecording) refresh();
      if (e.payload.recording !== undefined) wasRecording = e.payload.recording;
    });
    await listen<DownloadProgress>('daemon:downloads', (e) => daemon.update((s) => ({ ...s, downloading: e.payload.state === 'downloading' })));
    await listen<Down>('daemon:down', (e) => daemon.update((s) => ({ ...s, down: e.payload.reason })));
  });
</script>

<main aria-live="polite" style="display: flex; flex-direction: column;">
  <TitleBar />
  <Pad {latest} {landing} agent={null} />
  <Earlier rows={earlierRows} total={earlierTotal} />
  <span class="sr">{$announcement}</span>
</main>
