<script lang="ts">
  import { daemon, shownFloat } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { previewVoice, type Voices } from '../lib/tauri';
  import { announce } from '../lib/copy';
  import Field from '../controls/Field.svelte';
  import Segmented from '../controls/Segmented.svelte';

  export let voices: Voices = { voices: [], current: null };

  $: tts = ($daemon.status?.config?.tts ?? {}) as Record<string, unknown>;
  $: speed = shownFloat(Number(tts.speed ?? 1));
  // The config leads, so the mark moves to the voice a write just chose.
  $: current = String(tts.voice ?? voices.current ?? '');
  $: fallback = String(tts.fallback ?? 'system');
</script>

<Field name="Voice" pending={$daemon.pending.has('tts.voice')}>
  <div class="voices">
    {#each voices.voices as voice (voice.id)}
      {@const on = voice.id === current}
      <div class="voice" class:on>
        <input
          type="radio"
          name="voice"
          id={`voice-${voice.id}`}
          checked={on}
          on:change={() => write('tts.voice', voice.id)}
        />
        <label for={`voice-${voice.id}`}>
          <span class="name">{voice.name}</span>
          <span class="desc">{voice.description}</span>
        </label>
        <button
          class="btn btn-ghost"
          aria-label={`Preview ${voice.name}`}
          on:click={() =>
            previewVoice(voice.id).catch(() => announce('That voice will not play.'))}
        >
          Play
        </button>
      </div>
    {:else}
      <p class="empty">No voices are downloaded yet.</p>
    {/each}
  </div>
</Field>

<Field name="Speaking rate" pending={$daemon.pending.has('tts.speed')}>
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
</Field>

<Field name="If a voice is missing" pending={$daemon.pending.has('tts.fallback')}>
  <Segmented
    label="If a voice is missing"
    value={fallback}
    options={[
      { value: 'system', label: 'System voice' },
      { value: 'none', label: 'Silence' },
    ]}
    change={(next) => write('tts.fallback', next)}
  />
</Field>

<style>
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
    font-variation-settings: 'wght' 600, 'wdth' 100;
    font-size: 15px;
    width: 58px;
    flex: none;
  }

  .on .name {
    font-variation-settings: 'wght' 800, 'wdth' 105;
  }

  .desc {
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    margin: 0;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 13px;
  }
</style>
