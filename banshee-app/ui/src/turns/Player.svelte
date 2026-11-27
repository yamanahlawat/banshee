<script lang="ts">
  import { onDestroy } from 'svelte';
  import { report } from '../lib/copy';
  import { historyAudio, type HistoryRow } from '../lib/tauri';

  export let id: HistoryRow['id'];
  /// Counted by the daemon when the audio was kept, so the name is right before any bytes arrive.
  export let seconds: number;

  const CANNOT_PLAY = 'This machine cannot play the audio.';
  const DID_NOT_PLAY = 'The audio did not play.';

  let element: HTMLAudioElement;
  let url: string | null = null;
  let fetching = false;
  let playing = false;
  let elapsed = 0;
  let destroyed = false;

  $: whole = Math.max(1, Math.round(seconds));
  $: name = playing ? 'Pause audio' : `Play audio, ${whole} ${whole === 1 ? 'second' : 'seconds'}`;
  $: readout = clock(playing ? Math.floor(elapsed) : whole);

  function clock(total: number): string {
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
  }

  async function source(): Promise<string | null> {
    if (url) return url;
    fetching = true;
    try {
      const bytes = await historyAudio(id);
      if (destroyed) return null;
      url = URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' }));
      element.src = url;
      return url;
    } catch (error) {
      if (!destroyed) report((error as { message?: string })?.message || 'The audio did not load.');
      return null;
    } finally {
      fetching = false;
    }
  }

  export async function toggle() {
    if (playing) {
      element.pause();
      return;
    }
    // A press while the bytes are on their way would start the clip twice.
    if (fetching || !(await source())) return;
    try {
      await element.play();
    } catch (error) {
      const refusal = (error as DOMException)?.name;
      // A pause landing before playback starts rejects the play; that is no fault.
      if (refusal !== 'AbortError') {
        failed(refusal === 'NotSupportedError' ? CANNOT_PLAY : DID_NOT_PLAY);
      }
    }
  }

  function failed(message: string) {
    playing = false;
    report(message);
  }

  onDestroy(() => {
    destroyed = true;
    if (playing) element.pause();
    if (url) {
      URL.revokeObjectURL(url);
      url = null;
      if (element) {
        element.removeAttribute('src');
        element.load();
      }
    }
  });
</script>

<div class="player">
  <button class="btn btn-ghost play" on:click={toggle}>
    <span class="sr">{name}</span>
    <svg class="icon glyph" viewBox="0 0 12 12" aria-hidden="true">
      {#if playing}
        <rect x="3" y="2.5" width="1.6" height="7" />
        <rect x="7.4" y="2.5" width="1.6" height="7" />
      {:else}
        <path d="M3.8 2.4 L9.4 6 L3.8 9.6 Z" />
      {/if}
    </svg>
  </button>
  <span class="mono readout" aria-hidden="true">{readout}</span>
  <audio
    bind:this={element}
    preload="none"
    on:play={() => (playing = true)}
    on:pause={() => (playing = false)}
    on:ended={() => {
      playing = false;
      elapsed = 0;
    }}
    on:timeupdate={() => (elapsed = element.currentTime)}
    on:error={() => {
      // An error with no clip held belongs to one already let go.
      if (url) failed(CANNOT_PLAY);
    }}
  ></audio>
</div>

<style>
  .player {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 24px;
  }
  .play {
    display: inline-grid;
    place-items: center;
    width: 34px;
    height: 34px;
    padding: 0;
    flex: none;
  }
  .glyph {
    width: 12px;
    height: 12px;
    fill: currentColor;
  }
  .readout {
    font-size: 13px;
    color: var(--accent);
  }
</style>
