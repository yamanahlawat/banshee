<script lang="ts">
  import { daemon } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { previewVoice, type Voices } from '../lib/tauri';
  import { announce } from '../lib/copy';
  import Row from '../controls/Row.svelte';
  import More from '../controls/More.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import Slider from '../controls/Slider.svelte';

  export let voices: Voices = { voices: [], current: null };

  $: tts = ($daemon.status?.config?.tts ?? {}) as Record<string, unknown>;
  $: speed = Number(tts.speed ?? 1);
  // The config leads, so the mark moves to the voice a write just chose.
  $: current = String(tts.voice ?? voices.current ?? '');
</script>

<fieldset style="margin: 6px 0 0; padding: 0; border: 0; min-width: 0;">
  <legend class="sr">Voice</legend>
  <ul style="margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column;">
    {#each voices.voices as voice (voice.id)}
      {@const on = voice.id === current}
      <li style="display: flex; align-items: center; gap: 12px; min-height: 40px; padding: 4px 0; border-top: 1px solid var(--rule);">
        <input
          type="radio"
          name="voice"
          id={`voice-${voice.id}`}
          checked={on}
          onchange={() => write('tts.voice', voice.id)}
          style="width: 16px; height: 16px; margin: 0; accent-color: var(--ink); flex-shrink: 0;"
        />
        <label for={`voice-${voice.id}`} style="display: flex; align-items: baseline; gap: 10px; flex: 1; min-width: 0; cursor: pointer;">
          <span style="font-weight: {on ? 600 : 400}; width: 64px;">{voice.name}</span>
          <span style="flex: 1; color: var(--dim);">{voice.description}</span>
          <span class="mono" style="color: var(--dim);">{voice.id}</span>
        </label>
        <button
          type="button"
          aria-label={`Preview ${voice.name}`}
          onclick={() => previewVoice(voice.id).catch(() => announce('That voice will not play.'))}
          style="width: 28px; height: 28px; border-radius: 6px; border: 1.5px solid var(--ink); background: transparent; color: var(--ink); display: flex; align-items: center; justify-content: center; cursor: pointer; padding: 0; flex-shrink: 0;"
        >
          <svg width="11" height="11" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 2 L10 6 L3 10 Z" fill="currentColor" /></svg>
        </button>
      </li>
    {/each}
  </ul>
</fieldset>
{#if $daemon.pending.has('tts.voice')}
  <p style="margin: 6px 0 0; color: var(--dim);">
    <span class="caps">Pending</span> Dictation uses the voice above once Banshee restarts.
  </p>
{/if}

<Row name="Speed" command={`banshee config set tts.speed ${speed}`} pending={$daemon.pending.has('tts.speed')}>
  <Slider label="Speed" value={speed} min={0.5} max={2} step={0.1} change={(next) => write('tts.speed', next)} />
  <span class="mono" style="width: 32px; text-align: right;">{speed}x</span>
</Row>

<More />

<Row name="If a voice is missing" command={`banshee config set tts.fallback ${String(tts.fallback ?? 'system')}`} pending={$daemon.pending.has('tts.fallback')}>
  <Segmented
    label="If a voice is missing"
    active={String(tts.fallback ?? 'system')}
    options={[{ value: 'system', label: 'System voice' }, { value: 'none', label: 'Silence' }]}
    change={(next) => write('tts.fallback', next)}
  />
</Row>
