<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    daemon,
    deviceLabel,
    listeningFacts,
    shownFloat,
    waitsOnARestart,
    SYSTEM_DEVICE,
  } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { askForModels, fetching } from '../lib/downloads';
  import { PRESETS } from '../lib/presets';
  import { listDevices, listLanguages, type Devices, type Languages } from '../lib/tauri';
  import Row from '../controls/Row.svelte';
  import Field from '../controls/Field.svelte';
  import KeyRow from '../controls/KeyRow.svelte';
  import Picker from '../controls/Picker.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import ProviderGroup from '../controls/ProviderGroup.svelte';
  import SubRow from '../controls/SubRow.svelte';
  import Failure from '../controls/Failure.svelte';
  import { claimKeys } from '../lib/keys';
  import {
    announcer,
    languageNote as saysAboutLanguage,
    listeningNote,
    modelCost,
    report,
    DICTATION_FAILED,
    DICTATION_FAILURE,
  } from '../lib/copy';

  // The choice and its consequence are one reading, so the group names the
  // sentence its radiogroup is described by.
  const LISTENER_NOTE = 'listener-note';

  // Three words are all the window reads back. The midpoints are derived from the band edges, not
  // measured against a room.
  const BANDS = [
    { upTo: 1 / 3, word: 'Low', writes: 0.15 },
    { upTo: 2 / 3, word: 'Medium', writes: 0.5 },
    { upTo: Infinity, word: 'High', writes: 0.85 },
  ];
  const HEARD = BANDS.map((b) => ({ value: b.word, label: b.word }));
  const QUIET = [1000, 1500, 2500, 4000];
  // Whisper translates in one direction only: any language in, English out.
  const ANSWER = [
    { value: 'spoken', label: 'What I said' },
    { value: 'english', label: 'English' },
  ];
  const LISTENING = [
    { value: 'local', label: 'On this machine' },
    { value: 'remote', label: 'A remote server' },
  ];
  let devices: Devices = { devices: [], current: null };
  let spoken: Languages = { languages: [] };
  let adding = false;
  let field: HTMLInputElement | undefined;
  // Typing a word owns the keyboard; otherwise Escape would close the panel and Cmd+F open Find mid-word.
  let release: (() => void) | null = null;

  function beginAdding() {
    adding = true;
    release ??= claimKeys();
  }

  function stopAdding() {
    adding = false;
    release?.();
    release = null;
  }

  onDestroy(() => release?.());

  // Whisper's own list, so the window cannot offer a code the engine refuses.
  // It does not move while the panel is open.
  let languagesArrived = true;
  listLanguages()
    .then((got) => (spoken = got))
    .catch(() => (languagesArrived = false));

  // Ordered by request: a slow earlier read must not overwrite a later one.
  let reading = 0;
  function readDevices() {
    const mine = ++reading;
    listDevices()
      .then((d) => {
        if (mine === reading) devices = d;
      })
      .catch(() => {});
  }

  // The watchdog rebinds capture on its own, so the list follows the daemon.
  // Compared rather than named: Svelte depends on the whole store, and
  // enumeration costs 85-104ms on the daemon's one mutex.
  let seen = '';
  $: {
    const key = `${$daemon.live.audio_device}|${$daemon.live.missing_device}`;
    if (key !== seen) {
      seen = key;
      readDevices();
    }
  }

  $: if (adding && field) field.focus();

  $: stt = ($daemon.status?.config?.stt ?? {}) as Record<string, unknown>;
  $: threshold = shownFloat(Number(stt.vad_threshold ?? 0.5));
  $: band = BANDS.find((b) => threshold < b.upTo)?.word ?? 'High';
  $: silence = Number(stt.endpoint_silence_ms ?? 2500);
  // Keyed by word below, and a duplicate in a hand-edited config raises
  // Svelte's `each_key_duplicate` and renders no panel.
  $: vocabulary = [...new Set((stt.vocabulary ?? []) as string[])];
  $: preset = String(stt.preset ?? 'balanced');
  $: provider = String(stt.provider ?? 'local');
  $: remoteTable = (stt.remote ?? {}) as Record<string, unknown>;
  // The daemon says which listener is in force; `provider` above says only
  // which one was asked for. One sentence reads both, so the group cannot say
  // two things at once.
  $: listening = listeningFacts($daemon, $waitsOnARestart);
  $: listenerNote = listeningNote(listening);
  // The daemon sends the vocabulary as the remote request's prompt, so under a
  // remote listener these words leave the machine too.
  $: vocabularyNote = listening.remote
    ? 'Words Banshee should expect to hear. They go to the server with your audio.'
    : 'Words Banshee should expect to hear.';
  $: lastError = $daemon.live.last_error;
  // The missing list, not the blockers: dictation carries on with the loaded
  // model, so a heavier preset raises nothing and would leave this row silent.
  // Not `pending` either, which a restart empties while the file is still gone.
  $: absentModel =
    ($daemon.status?.missing_downloads ?? []).find((one) => one.role === 'speech') ?? null;
  $: presetNeedsFetching = provider === 'local' && absentModel !== null;
  // No guard here: `askForModels` claims the run before it awaits anything.
  async function fetchModel() {
    await askForModels().catch(() => report('The download did not start.'));
  }
  $: presetSays = absentModel
    ? modelCost(presetName, absentModel.megabytes, Number($daemon.status?.download_megabytes ?? 0))
    : null;

  // Three fields appear or leave with no event of their own, and where the
  // audio goes changes with them, so a reader who is not looking hears the
  // consequence before the layout.
  const sawProvider = announcer<string>();
  $: if ($daemon.status) {
    sawProvider(
      provider,
      `${listenerNote} ${provider === 'remote' ? 'Server, key and model are below.' : 'Model is below.'}`,
    );
  }
  $: language = String(stt.language ?? 'en');
  $: translate = stt.translate === true;
  // The daemon's own word, so the preset name is not a second rule for one fact.
  // It answers for the model the listener loaded, which lags the preset beside
  // it while a heavier one is read off disk.
  $: englishOnly = $daemon.status?.english_only === true;
  $: loadingModel = $daemon.live.loading_model;
  $: presetName = PRESETS.find((one) => one.value === preset)?.label ?? preset;
  // A code Whisper's table does not name still needs its row: a select whose value matches no
  // option draws empty.
  $: languagesOffered =
    language === 'auto' || spoken.languages.some((one) => one.code === language)
      ? spoken.languages
      : [{ code: language, name: language }, ...spoken.languages];

  $: languageNote = saysAboutLanguage({
    languagesArrived,
    englishOnly,
    loading: loadingModel,
    chosen: presetName,
    isFast: preset === 'fast',
  });

  // `endpoint_silence_ms` is a plain u64 in the daemon, so a hand-edited config
  // can hold a value none of these offer.
  $: offered = QUIET.includes(silence) ? QUIET : [silence, ...QUIET];

  function quietFor(ms: number): string {
    return `After ${ms / 1000} second${ms === 1000 ? '' : 's'} of quiet`;
  }
  // The config is what a write changes, so it leads.
  $: current = String(
    ($daemon.status?.config?.audio?.input_device as string) ??
      $daemon.live.audio_device ??
      SYSTEM_DEVICE,
  );
  // An unplugged microphone still needs its row, or it reads as a missing
  // control.
  $: names = devices.devices.map((d) => d.name);
  $: options = names.includes(current) || current === SYSTEM_DEVICE ? names : [current, ...names];

  function addWord(raw: string) {
    stopAdding();
    const word = raw.trim();
    if (word === '' || vocabulary.includes(word)) return;
    write('stt.vocabulary', [...vocabulary, word]);
  }
</script>

<Row name="Input" block pending={$daemon.pending.has('audio.input_device')}>
  <Picker label="Input device" value={current} change={(next) => write('audio.input_device', next)}>
    <option value={SYSTEM_DEVICE}>{deviceLabel($daemon.live.audio_device)}</option>
    {#each options as name (name)}
      <option value={name}>{name}</option>
    {/each}
  </Picker>
</Row>

<Row name="Sensitivity" pending={$daemon.pending.has('stt.vad_threshold')}>
  <Segmented
    label="Sensitivity"
    value={band}
    options={HEARD}
    change={(next) => write('stt.vad_threshold', BANDS.find((b) => b.word === next)?.writes ?? 0.5)}
  />
</Row>

<Row name="End of speech" block pending={$daemon.pending.has('stt.endpoint_silence_ms')}>
  <Picker
    label="End of speech"
    value={String(silence)}
    change={(next) => write('stt.endpoint_silence_ms', Number(next))}
  >
    {#each offered as ms (ms)}
      <option value={String(ms)}>{quietFor(ms)}</option>
    {/each}
  </Picker>
</Row>

<ProviderGroup
  name="Listening"
  label="Listening"
  value={provider}
  options={LISTENING}
  note={listenerNote}
  noteId={LISTENER_NOTE}
  alsoId={lastError ? DICTATION_FAILURE : undefined}
  change={(next) => write('stt.provider', next)}
>
  {#if provider === 'remote'}
    <SubRow name="server" pending={$waitsOnARestart.has('stt.remote.base_url')}>
      <Field
        label="Server"
        value={String(remoteTable.base_url ?? '')}
        placeholder="https://api.openai.com/v1"
        commit={(next) => write('stt.remote.base_url', next)}
      />
    </SubRow>
    <KeyRow setting="stt.remote.api_key" present={listening.keyPresent} />
    <SubRow name="model" pending={$waitsOnARestart.has('stt.remote.model')}>
      <Field
        label="Model"
        value={String(remoteTable.model ?? '')}
        placeholder="whisper-1"
        commit={(next) => write('stt.remote.model', next)}
      />
    </SubRow>
  {:else}
    <SubRow name="model" pending={presetNeedsFetching} says={presetSays}>
      <Segmented
        label="Model"
        value={preset}
        options={PRESETS}
        change={(next) => write('stt.preset', next)}
      />
      <svelte:fragment slot="action">
        {#if presetNeedsFetching}
          <button class="btn fetch" disabled={$fetching} on:click={fetchModel}>
            {$fetching ? 'Downloading' : 'Download'}
          </button>
        {/if}
      </svelte:fragment>
    </SubRow>
  {/if}

  {#if lastError}
    <Failure id={DICTATION_FAILURE} label={DICTATION_FAILED} said={lastError} />
  {/if}
</ProviderGroup>

<!-- Beside the preset it depends on: the English-only model rules every other
     language out, and the two read as one decision only if they sit together. -->
<Row name="Language" block note={languageNote} pending={$waitsOnARestart.has('stt.language')}>
  <Picker
    label="Language"
    value={language}
    disabled={englishOnly && preset === 'fast'}
    change={(next) => write('stt.language', next)}
  >
    <!-- `auto` is a value the config takes and the engine reads as detect it,
         so it belongs in the list a person picks from. Whisper's own table
         holds only real languages. -->
    <option value="auto">Detect it</option>
    {#each languagesOffered as option (option.code)}
      <option value={option.code}>{option.name}</option>
    {/each}
  </Picker>
</Row>

{#if !englishOnly && language !== 'en'}
  <Row name="Answer in" pending={$waitsOnARestart.has('stt.translate')}>
    <Segmented
      label="Answer in"
      value={translate ? 'english' : 'spoken'}
      options={ANSWER}
      change={(next) => write('stt.translate', next === 'english')}
    />
  </Row>
{/if}

<Row name="Vocabulary" block note={vocabularyNote} pending={$daemon.pending.has('stt.vocabulary')}>
  <div class="chips">
    {#each vocabulary as word (word)}
      <span class="chip">
        {word}
        <button
          aria-label={`Remove ${word}`}
          on:click={() =>
            write(
              'stt.vocabulary',
              vocabulary.filter((w) => w !== word),
            )}>&times;</button
        >
      </span>
    {/each}
    {#if adding}
      <input
        bind:this={field}
        class="chip add"
        aria-label="New word"
        on:blur={(e) => addWord(e.currentTarget.value)}
        on:keydown={(e) => {
          if (e.key === 'Enter') addWord(e.currentTarget.value);
          if (e.key === 'Escape') {
            // The window's own Escape closes the panel, and this event still
            // reaches it: the claim is gone by the time it bubbles.
            e.stopPropagation();
            stopAdding();
          }
        }}
      />
    {:else}
      <button class="btn btn-ghost" on:click={beginAdding}>Add a word</button>
    {/if}
  </div>
</Row>

<style>
  /* Under the sentence that explains it, aligned with the control it acts for. */
  .fetch {
    grid-column: 2;
    justify-self: start;
    margin-top: 12px;
  }

  /* Centred, not stretched: a wrapped flex line sizes its items to the tallest, so the input would
     stretch the chips beside it. */
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    width: 100%;
  }

  .add {
    background: transparent;
    width: 110px;
  }
</style>
