<script lang="ts">
  import { daemon, speechFacts } from '../lib/daemon';
  import {
    defaultOf,
    optionsOf,
    refusalId,
    refuse,
    refused,
    rowOf,
    table,
    write,
  } from '../lib/settings';
  import { previewVoice, type Voice, type Voices } from '../lib/tauri';
  import {
    blocked,
    BUSY_NOTE,
    claimFor,
    fetching,
    forgetTheAsk,
    runTheDownload,
  } from '../lib/downloads';
  import {
    announce,
    announcer,
    report,
    speechNote,
    SPEECH_FAILED,
    SPEECH_FAILURE,
  } from '../lib/copy';
  import Field from '../controls/Field.svelte';
  import KeyRow from '../controls/KeyRow.svelte';
  import ProviderGroup from '../controls/ProviderGroup.svelte';
  import Row from '../controls/Row.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import Failure from '../controls/Failure.svelte';
  import DownloadLine from '../controls/DownloadLine.svelte';
  import BusyNote from '../controls/BusyNote.svelte';
  import Refusal from '../controls/Refusal.svelte';
  import { land } from '../lib/focus';
  import { travel, watchGroups } from '../lib/travel';
  import { idList } from '../lib/ids';

  export let voices: Voices = { voices: [], current: null };

  // The choice and its consequence are one reading, so the group names the
  // sentence its radiogroup is described by.
  const SPEAKER_NOTE = 'speaker-note';

  $: tts = ($daemon.status?.config?.tts ?? {}) as Record<string, unknown>;
  // The config leads, so the mark moves to the voice a write just chose.
  $: current = String(tts.voice ?? voices.current ?? '');
  $: provider = String(tts.provider ?? defaultOf($table, 'tts.provider'));
  $: remoteTable = (tts.remote ?? {}) as Record<string, unknown>;
  $: format = String(
    remoteTable.response_format ?? defaultOf($table, 'tts.remote.response_format'),
  );
  // The daemon says which speaker is in force; `provider` above says only which
  // one was asked for.
  $: speech = speechFacts($daemon, voices.voices);
  $: speakerNote = speechNote(speech);

  $: lastError = $daemon.live.last_speech_error;

  const here = (voice: Voice) => voice.downloaded !== false;
  // The current voice stays a choice while its file is absent, so the group is
  // never empty while a voice is set.
  const choosable = (voice: Voice, current: string) => here(voice) || voice.id === current;
  $: choices = voices.voices.filter((voice) => choosable(voice, current));
  $: toGet = voices.voices.filter((voice) => !choosable(voice, current));
  $: currentHere = voices.voices.every((voice) => voice.id !== current || here(voice));

  // A voice that changes group travels.
  const moved = watchGroups();
  $: {
    const key = moved(
      voices.voices.map((voice) => ({
        key: voice.id,
        group: choosable(voice, current) ? 'voice' : 'more',
      })),
    );
    if (key !== null) travel(key);
  }

  // A refused write leaves the voice where it was asked for, and its refusal
  // is said on that row alone.
  let asked: string | null = null;
  $: refusedUnderMore = toGet.some((voice) => voice.id === asked);
  $: refusedGet = refusedUnderMore && $refused.has('tts.voice') ? asked : null;

  let awaited: string | null = null;
  $: if (awaited !== null) {
    const landed = voices.voices.find(
      (voice) => voice.id === awaited && voice.downloaded !== false,
    );
    if (landed) {
      announce(`${landed.name} is ready.`);
      awaited = null;
    }
  }

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
    const domain = rowOf($table, 'tts.remote.sample_rate')?.domain;
    const outside =
      domain?.kind === 'number' &&
      (Number(rate) < domain.min || (domain.max !== null && Number(rate) > domain.max));
    if (!/^\d+$/.test(rate) || outside) {
      const message = 'The sample rate is a whole number of hertz, as in 24000.';
      refuse('tts.remote.sample_rate', message);
      return false;
    }
    return write('tts.remote.sample_rate', Number(rate));
  }

  async function select(voice: Voice): Promise<boolean> {
    asked = voice.id;
    if (await write('tts.voice', voice.id)) return true;
    // A refusal leaves `current` where it was, so nothing renders the mark back
    // from where the browser moved it. Checking one clears the group.
    const loaded = document.getElementById(`voice-${current}`);
    if (loaded instanceof HTMLInputElement) loaded.checked = true;
    return false;
  }

  // A run fetches whatever the config asks for, so a second voice pressed into
  // it would be written down and left out of the run already going.
  async function fetchVoice(voice: Voice) {
    if (!claimFor(voice.id)) return;
    const moving = voice.id !== current;
    if (!(await select(voice))) {
      forgetTheAsk();
      return;
    }
    awaited = voice.id;
    announce(`Getting ${voice.name}.`);
    if (moving) await land(() => document.getElementById(`voice-${voice.id}`));
    await runTheDownload().catch(() =>
      report(`${voice.name} did not download. Press Get to try again.`),
    );
  }
</script>

<ProviderGroup
  name="Speaking"
  label="Speaking"
  value={provider}
  options={optionsOf($table, 'tts.provider')}
  note={speakerNote}
  noteId={SPEAKER_NOTE}
  alsoId={lastError ? SPEECH_FAILURE : undefined}
  setting="tts.provider"
  change={(next) => write('tts.provider', next)}
>
  {#if provider === 'remote'}
    <Row name="Server" setting="tts.remote.base_url">
      <Field
        label="Server"
        value={String(remoteTable.base_url ?? '')}
        placeholder="https://api.openai.com/v1"
        commit={(next) => write('tts.remote.base_url', next)}
      />
    </Row>
    <KeyRow setting="tts.remote.api_key" present={speech.keyPresent} />
    <Row name="Model" setting="tts.remote.model">
      <Field
        label="Model"
        value={String(remoteTable.model ?? '')}
        placeholder="tts-1"
        commit={(next) => write('tts.remote.model', next)}
      />
    </Row>
    <Row name="Voice" setting="tts.remote.voice">
      <Field
        label="Voice"
        value={String(remoteTable.voice ?? '')}
        dashed={String(remoteTable.voice ?? '') === ''}
        placeholder="A voice the server names"
        commit={(next) => write('tts.remote.voice', next)}
      />
    </Row>
    <Row name="Tone" setting="tts.remote.instructions">
      <Field
        label="Tone"
        value={String(remoteTable.instructions ?? '')}
        placeholder="Optional"
        commit={(next) => write('tts.remote.instructions', next)}
      />
    </Row>
    <Row name="Format" setting="tts.remote.response_format">
      <Segmented
        label="Audio format"
        value={format}
        options={optionsOf($table, 'tts.remote.response_format')}
        change={(next) => write('tts.remote.response_format', next)}
      />
    </Row>
    {#if format === 'pcm'}
      <Row name="Sample rate" setting="tts.remote.sample_rate">
        <Field
          label="Sample rate"
          value={remoteTable.sample_rate == null ? '' : String(remoteTable.sample_rate)}
          placeholder="24000"
          commit={writeRate}
        />
      </Row>
    {/if}
  {:else}
    <Row name="Voice" setting="tts.voice" refusal={!refusedUnderMore} let:describedBy>
      <div class="voices" role="radiogroup" aria-label="Voice" aria-describedby={describedBy}>
        {#each choices as voice (voice.id)}
          {@const on = voice.id === current}
          {@const fetched = here(voice)}
          <div class="voice choice-row" class:on data-travel={voice.id}>
            <input
              type="radio"
              name="voice"
              id={`voice-${voice.id}`}
              checked={on}
              on:change={() => select(voice)}
            />
            <label class="words" for={`voice-${voice.id}`}>
              <span class="name" class:absent={!fetched}>{voice.name}</span>
              <span class="desc note">{voice.description}</span>
              <!-- The dashed name is the mark for the eye, and this is the same
                   fact for a reader who hears the radio rather than sees it. -->
              {#if !fetched}<span class="sr">, not on this machine</span>{/if}
            </label>
            <!-- One button, so focus stays on it when the file lands. -->
            <button
              class="btn btn-ghost pick"
              aria-label={fetched
                ? `Play ${voice.name}`
                : `Get ${voice.name}, which is not on this machine`}
              on:click={() =>
                fetched
                  ? previewVoice(voice.id).catch(() => report(`${voice.name} will not play.`))
                  : fetchVoice(voice)}
            >
              {fetched ? 'Play' : 'Get'}
            </button>
          </div>
        {:else}
          <p class="empty note-type">
            {speech.live
              ? "No voices yet. They arrive with Banshee's models."
              : 'Banshee is not running, so it cannot list its voices.'}
          </p>
        {/each}
      </div>
      <svelte:fragment slot="action">
        {#if !currentHere && $daemon.download}<DownloadLine />{/if}
      </svelte:fragment>
    </Row>
    {#if toGet.length > 0}
      <Row
        name="More voices"
        note="Get downloads a voice and switches to it."
        leaves
        let:describedBy
      >
        <ul class="voices" role="list" aria-label="More voices" aria-describedby={describedBy}>
          {#each toGet as voice (voice.id)}
            {@const refusedHere = voice.id === refusedGet}
            {@const blockedHere = voice.id === $blocked}
            <li class="voice choice-row" data-travel={voice.id}>
              <span class="mark" aria-hidden="true"></span>
              <span class="words">
                <span class="name">{voice.name}</span>
                <span class="desc note">{voice.description}</span>
              </span>
              <button
                class="btn btn-ghost pick"
                aria-label={`Get ${voice.name} and switch to it`}
                aria-describedby={idList(
                  describedBy,
                  refusedHere && refusalId('tts.voice'),
                  blockedHere && BUSY_NOTE,
                )}
                aria-disabled={$fetching ? 'true' : undefined}
                on:click={() => fetchVoice(voice)}
              >
                Get
              </button>
              {#if refusedHere || blockedHere}
                <div class="said">
                  {#if refusedHere}<Refusal setting="tts.voice" />{/if}
                  <BusyNote button={voice.id} />
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      </Row>
    {/if}
  {/if}

  {#if lastError}
    <Failure id={SPEECH_FAILURE} label={SPEECH_FAILED} said={lastError} />
  {/if}
</ProviderGroup>

<!-- `tts.fallback` is deliberately absent: it serves no job this audience has.
     It stays in config.toml and the CLI. -->

<style>
  /* The dash this world uses for a thing that is not here yet. */
  .absent {
    border-bottom: 1px dashed var(--accent);
  }

  .voices {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .voice {
    padding: 7px 0;
    min-width: 0;
  }

  .said {
    grid-column: 2 / span 2;
  }

  /* A description too long for the line drops under the name whole, so a
     licence credit is never cut. */
  .words {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    column-gap: 10px;
    min-width: 0;
  }

  label.words {
    cursor: pointer;
  }

  /* Wide enough for the longest name the daemon ships, so every quality word
     starts at one edge down the list. */
  .name {
    --wght: var(--cut-name-weight);
    --wdth: var(--cut-name-width);
    font-size: 15px;
    width: 88px;
    flex: none;
  }

  /* One left edge for Play and Get, so the only strong vertical in the block
     does not step in and out as the list changes. */
  .pick {
    grid-column: 3;
    width: 62px;
  }

  .on .name {
    --wght: var(--cut-head-weight);
    --wdth: var(--cut-head-width);
  }

  .empty {
    max-width: 440px;
    margin: 0;
  }
</style>
