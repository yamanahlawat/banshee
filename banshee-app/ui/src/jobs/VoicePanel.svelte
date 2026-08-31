<script lang="ts">
  import { daemon, shownFloat, waitsOnARestart } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { downloadModels, previewVoice, type Voice, type Voices } from '../lib/tauri';
  import { report } from '../lib/copy';
  import Row from '../controls/Row.svelte';

  export let voices: Voices = { voices: [], current: null };

  $: tts = ($daemon.status?.config?.tts ?? {}) as Record<string, unknown>;
  $: speed = shownFloat(Number(tts.speed ?? 1));
  // The config leads, so the mark moves to the voice a write just chose.
  $: current = String(tts.voice ?? voices.current ?? '');

  // The daemon names every voice it can describe, and says which are here. A
  // voice that is not costs 510 KB, and the daemon applies it once the file
  // lands, so choosing one is the whole of the interaction.
  async function choose(voice: Voice, here: boolean) {
    await write('tts.voice', voice.id);
    if (!here) {
      await downloadModels().catch(() => report(`${voice.name} would not download.`));
    }
  }
</script>

<Row name="Voice" block pending={$waitsOnARestart.has('tts.voice')}>
  <div class="voices">
    {#each voices.voices as voice (voice.id)}
      {@const on = voice.id === current}
      {@const here = voice.downloaded !== false}
      <div class="voice" class:on>
        <input
          type="radio"
          name="voice"
          id={`voice-${voice.id}`}
          checked={on}
          on:change={() => choose(voice, here)}
        />
        <label for={`voice-${voice.id}`}>
          <span class="name" class:absent={!here}>{voice.name}</span>
          <span class="desc">{voice.description}</span>
          {#if !here}<span class="sr">— not downloaded, 510 KB</span>{/if}
        </label>
        <button
          class="btn btn-ghost"
          aria-label={`Preview ${voice.name}`}
          disabled={!here}
          on:click={() =>
            previewVoice(voice.id).catch(() => report(`${voice.name} will not play.`))}
        >
          Play
        </button>
      </div>
    {:else}
      <p class="empty">No voices yet. They arrive with Banshee's models.</p>
    {/each}
  </div>
</Row>

<Row name="Speaking rate" pending={$waitsOnARestart.has('tts.speed')}>
  <input
    class="range"
    type="range"
    aria-label="Speaking rate"
    min="0.5"
    max="2"
    step="0.1"
    value={speed}
    on:change={(e) => write('tts.speed', Number(e.currentTarget.value))}
  />
  <span class="readout">{speed}&times;</span>
</Row>

<!-- `tts.fallback` is deliberately absent: it serves no job this audience has.
     It stays in config.toml and the CLI. -->

<style>
  /* The dash this world uses for a thing that is not here yet. Choosing the
     voice fetches it. */
  .absent {
    border-bottom: 1px dashed var(--accent);
  }

  .voices {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
  }

  .voice {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 0;
    min-width: 0;
  }

  .voice input {
    margin: 0;
    flex: none;
  }

  .voice label {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex: 1;
    min-width: 0;
    cursor: pointer;
  }

  .name {
    font-variation-settings:
      'wght' 600,
      'wdth' 100;
    font-size: 15px;
    width: 58px;
    flex: none;
  }

  .on .name {
    font-variation-settings:
      'wght' 800,
      'wdth' 105;
  }

  .desc {
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    margin: 0;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
  }
</style>
