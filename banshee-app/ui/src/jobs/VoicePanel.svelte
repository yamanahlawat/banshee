<script lang="ts">
  import { daemon, shownFloat, speechFacts, waitsOnARestart } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { downloadModels, previewVoice, type Voice, type Voices } from '../lib/tauri';
  import {
    announce,
    announcer,
    report,
    speechNote,
    SPEECH_FAILED,
    SPEECH_FAILURE,
  } from '../lib/copy';
  import Row from '../controls/Row.svelte';
  import Field from '../controls/Field.svelte';
  import KeyRow from '../controls/KeyRow.svelte';
  import ProviderGroup from '../controls/ProviderGroup.svelte';
  import SubRow from '../controls/SubRow.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import Failure from '../controls/Failure.svelte';

  export let voices: Voices = { voices: [], current: null };

  // The choice and its consequence are one reading, so the group names the
  // sentence its radiogroup is described by.
  const SPEAKER_NOTE = 'speaker-note';
  const SPEAKING = [
    { value: 'local', label: 'On this machine' },
    { value: 'remote', label: 'A remote server' },
  ];
  const FORMATS = [
    { value: 'wav', label: 'WAV' },
    { value: 'pcm', label: 'PCM' },
  ];

  $: tts = ($daemon.status?.config?.tts ?? {}) as Record<string, unknown>;
  $: speed = shownFloat(Number(tts.speed ?? 1.2));
  // The config leads, so the mark moves to the voice a write just chose.
  $: current = String(tts.voice ?? voices.current ?? '');
  $: provider = String(tts.provider ?? 'local');
  $: remoteTable = (tts.remote ?? {}) as Record<string, unknown>;
  $: format = String(remoteTable.response_format ?? 'wav');
  // The daemon says which speaker is in force; `provider` above says only which
  // one was asked for.
  $: speech = speechFacts($daemon, $waitsOnARestart, voices.voices);
  $: speakerNote = speechNote(speech);

  $: lastError = $daemon.live.last_speech_error;

  // Fields appear or leave with no event of their own, and where the text
  // goes changes with them, so a reader who is not looking hears the
  // consequence before the layout.
  const sawProvider = announcer<string>();
  $: if ($daemon.status) {
    sawProvider(
      provider,
      `${speakerNote} ${provider === 'remote' ? 'Server, key, model, voice and format are below.' : 'The voices are below.'}`,
    );
  }

  // Empty is null, so the daemon goes back to the rate it assumes.
  function writeRate(typed: string): Promise<boolean> | boolean {
    const rate = typed.trim();
    if (rate === '') return write('tts.remote.sample_rate', null);
    // The bound is the config type's own: a NonZero<u32>.
    if (!/^\d+$/.test(rate) || Number(rate) < 1 || Number(rate) > 4_294_967_295) {
      announce('The sample rate is a whole number of hertz, as in 24000.');
      return false;
    }
    return write('tts.remote.sample_rate', Number(rate));
  }

  // The daemon applies a voice once its file lands, so choosing one is the whole interaction.
  async function choose(voice: Voice, here: boolean) {
    if (!(await write('tts.voice', voice.id))) {
      // A refusal leaves `current` where it was, so nothing renders the mark
      // back from where the browser moved it. Checking one clears the group.
      const loaded = document.getElementById(`voice-${current}`);
      if (loaded instanceof HTMLInputElement) loaded.checked = true;
      return;
    }
    if (!here) {
      await downloadModels().catch(() => report(`${voice.name} would not download.`));
    }
  }
</script>

<ProviderGroup
  name="Speaking"
  label="Speaking"
  value={provider}
  options={SPEAKING}
  note={speakerNote}
  noteId={SPEAKER_NOTE}
  alsoId={lastError ? SPEECH_FAILURE : undefined}
  change={(next) => write('tts.provider', next)}
>
  {#if provider === 'remote'}
    <SubRow name="server" pending={$waitsOnARestart.has('tts.remote.base_url')}>
      <Field
        label="Server"
        value={String(remoteTable.base_url ?? '')}
        placeholder="https://api.openai.com/v1"
        commit={(next) => write('tts.remote.base_url', next)}
      />
    </SubRow>
    <KeyRow setting="tts.remote.api_key" present={speech.keyPresent} />
    <SubRow name="model" pending={$waitsOnARestart.has('tts.remote.model')}>
      <Field
        label="Model"
        value={String(remoteTable.model ?? '')}
        placeholder="tts-1"
        commit={(next) => write('tts.remote.model', next)}
      />
    </SubRow>
    <SubRow name="voice" pending={$waitsOnARestart.has('tts.remote.voice')}>
      <Field
        label="Voice"
        value={String(remoteTable.voice ?? '')}
        dashed={String(remoteTable.voice ?? '') === ''}
        placeholder="A voice the server names"
        commit={(next) => write('tts.remote.voice', next)}
      />
    </SubRow>
    <SubRow name="tone" pending={$waitsOnARestart.has('tts.remote.instructions')}>
      <Field
        label="Tone"
        value={String(remoteTable.instructions ?? '')}
        placeholder="Optional"
        commit={(next) => write('tts.remote.instructions', next)}
      />
    </SubRow>
    <SubRow name="format" pending={$waitsOnARestart.has('tts.remote.response_format')}>
      <Segmented
        label="Audio format"
        value={format}
        options={FORMATS}
        change={(next) => write('tts.remote.response_format', next)}
      />
    </SubRow>
    {#if format === 'pcm'}
      <SubRow name="rate" pending={$waitsOnARestart.has('tts.remote.sample_rate')}>
        <Field
          label="Sample rate"
          value={remoteTable.sample_rate == null ? '' : String(remoteTable.sample_rate)}
          placeholder="24000"
          commit={writeRate}
        />
      </SubRow>
    {/if}
  {:else}
    <SubRow name="voice" pending={$waitsOnARestart.has('tts.voice')}>
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
    </SubRow>
  {/if}

  {#if lastError}
    <Failure id={SPEECH_FAILURE} label={SPEECH_FAILED} said={lastError} />
  {/if}
</ProviderGroup>

<!-- Outside the group: the rate applies to whichever speaker is in force. -->
<Row name="Speaking rate" centred pending={$waitsOnARestart.has('tts.speed')}>
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
