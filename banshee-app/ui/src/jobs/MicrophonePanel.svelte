<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    chosenWaitsOnLicence,
    daemon,
    deviceLabel,
    listeningFacts,
    shownFloat,
    waitsOnARestart,
    SYSTEM_DEVICE,
  } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { askForModels, fetching } from '../lib/downloads';
  import { listDevices, type Devices } from '../lib/tauri';
  import Row from '../controls/Row.svelte';
  import Field from '../controls/Field.svelte';
  import KeyRow from '../controls/KeyRow.svelte';
  import Picker from '../controls/Picker.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import ProviderGroup from '../controls/ProviderGroup.svelte';
  import SubRow from '../controls/SubRow.svelte';
  import Failure from '../controls/Failure.svelte';
  import LicenceWait from '../controls/LicenceWait.svelte';
  import { claimKeys, onMac } from '../lib/keys';
  import {
    announcer,
    listeningNote,
    modelCost,
    modelFacts,
    readLicence,
    report,
    DICTATION_FAILED,
    DICTATION_FAILURE,
  } from '../lib/copy';
  import type { ListeningModel } from '../lib/daemon';

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
  // A model translates in one direction only: any language in, English out.
  const WRITE_IN = [
    { value: 'spoken', label: 'The language spoken' },
    { value: 'english', label: 'English' },
  ];
  const mac = onMac();
  const LISTENING = [
    { value: 'local', label: 'On this machine' },
    { value: 'remote', label: 'A transcription server' },
  ];
  const OPENS = [
    { value: 'use', label: 'When you speak' },
    { value: 'always', label: 'Always' },
  ];
  // `models.unload_after` counts whole minutes and 0 is never, so no choice
  // unloads at the end of a use.
  const UNLOAD = [0, 2, 5, 10, 15, 60];
  const MORE = 'microphone-more';
  let more = false;
  let devices: Devices = { devices: [], current: null };
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
  $: models = $daemon.status?.stt_models ?? [];
  $: chosenId = String(stt.model ?? '');
  $: chosen = models.find((one) => one.id === chosenId) ?? null;
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
  $: waitsOnLicence = provider === 'local' ? chosenWaitsOnLicence(models, chosenId) : null;
  // The model's own entry, not the blockers: dictation carries on with the
  // loaded model, so a chosen one that is absent raises nothing. Not `pending`
  // either, which a restart empties while the file is still gone.
  $: needsFetching =
    provider === 'local' && chosen !== null && !chosen.downloaded && waitsOnLicence === null;
  // No guard here: `askForModels` claims the run before it awaits anything.
  async function fetchModel() {
    await askForModels().catch(() => report('The download did not start.'));
  }
  $: modelSays =
    needsFetching && chosen
      ? modelCost(chosen.label, chosen.megabytes, Number($daemon.status?.download_megabytes ?? 0))
      : null;

  async function choose(model: ListeningModel) {
    if (await write('stt.model', model.id)) return;
    // A refusal leaves `chosenId` where it was, so nothing renders the mark
    // back from where the browser moved it.
    const held = document.getElementById(`model-${chosenId}`);
    if (held instanceof HTMLInputElement) held.checked = true;
  }

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
  $: translate = stt.translate === true;
  // The catalogue's word for the model in force, known before any load.
  $: translates = $daemon.status?.stt_translates === true;

  // `endpoint_silence_ms` is a plain u64 in the daemon, so a hand-edited config
  // can hold a value none of these offer.
  $: offered = QUIET.includes(silence) ? QUIET : [silence, ...QUIET];

  $: audio = ($daemon.status?.config?.audio ?? {}) as Record<string, unknown>;
  $: keepOpen = audio.keep_open === true;
  $: openNote = keepOpen
    ? 'The microphone stays open, so recording starts at once. The system shows the microphone in use, and a Bluetooth headset plays at phone-call quality.'
    : 'Banshee opens the microphone when you speak and closes it right after.';
  $: unloadAfter = Number($daemon.status?.config?.models?.unload_after ?? 5);
  // A hand-edited config can hold a value none of these offer.
  $: unloadOffered = UNLOAD.includes(unloadAfter) ? UNLOAD : [unloadAfter, ...UNLOAD];
  $: unloadNote =
    unloadAfter === 0
      ? 'The listening model and the voice stay in memory.'
      : 'The listening model and the voice leave memory when Banshee has been idle this long. The next use loads them again.';

  function idleFor(minutes: number): string {
    if (minutes === 0) return 'Never';
    if (minutes === 60) return 'After 1 hour idle';
    return `After ${minutes} minute${minutes === 1 ? '' : 's'} idle`;
  }

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
  {:else if models.length > 0}
    <SubRow name="model" pending={modelSays !== null} says={modelSays}>
      <div class="models" role="radiogroup" aria-label="Listening model">
        {#each models as model (model.id)}
          <div class="model" class:on={model.id === chosenId}>
            <input
              type="radio"
              name="listening-model"
              id={`model-${model.id}`}
              checked={model.id === chosenId}
              aria-describedby={`model-${model.id}-facts`}
              on:change={() => choose(model)}
            />
            <!-- The dashed name is the mark for the eye; the hidden words are the
                 same fact for a reader who hears the radio. -->
            <label for={`model-${model.id}`}
              ><span class="name" class:absent={!model.downloaded}>{model.label}</span
              >{#if !model.downloaded}<span class="sr">, not on this machine</span>{/if}</label
            >
            <p class="facts" id={`model-${model.id}-facts`}>{modelFacts(model, mac)}</p>
            {#if model.credit}
              <p class="credit">
                {model.credit}
                {#if waitsOnLicence?.id !== model.id}
                  <button
                    class="caps btn-underline"
                    aria-label={`Read the licence for ${model.label}`}
                    on:click={() => readLicence(model)}>Read the licence</button
                  >
                {/if}
              </p>
            {/if}
          </div>
        {/each}
      </div>
      <svelte:fragment slot="action">
        {#if waitsOnLicence}
          <div class="licence">
            <LicenceWait model={waitsOnLicence} />
          </div>
        {:else if needsFetching}
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

{#if translates}
  <Row name="Write it in" pending={$waitsOnARestart.has('stt.translate')}>
    <Segmented
      label="Write it in"
      value={translate ? 'english' : 'spoken'}
      options={WRITE_IN}
      change={(next) => write('stt.translate', next === 'english')}
    />
  </Row>
{/if}

<Row
  name="Words to know"
  block
  note={vocabularyNote}
  pending={$daemon.pending.has('stt.vocabulary')}
>
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

<button
  class="caps btn-underline more"
  aria-label="More listening settings"
  aria-expanded={more}
  aria-controls={MORE}
  on:click={() => (more = !more)}
>
  More
  <svg width="11" height="7" viewBox="0 0 11 7" aria-hidden="true" focusable="false">
    <path
      d="M1 1.5 L5.5 5.5 L10 1.5"
      fill="none"
      stroke="currentColor"
      stroke-width="1.4"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
</button>

<div id={MORE}>
  {#if more}
    <Row
      name="Microphone"
      note={openNote}
      noteId="microphone-opens-note"
      pending={$daemon.pending.has('audio.keep_open')}
    >
      <Segmented
        label="Microphone"
        value={keepOpen ? 'always' : 'use'}
        options={OPENS}
        describedBy="microphone-opens-note"
        change={(next) => write('audio.keep_open', next === 'always')}
      />
    </Row>

    <Row
      name="Free memory"
      block
      note={unloadNote}
      pending={$daemon.pending.has('models.unload_after')}
    >
      <Picker
        label="Free memory"
        value={String(unloadAfter)}
        change={(next) => write('models.unload_after', Number(next))}
      >
        {#each unloadOffered as minutes (minutes)}
          <option value={String(minutes)}>{idleFor(minutes)}</option>
        {/each}
      </Picker>
    </Row>
  {/if}
</div>

<style>
  /* Under the sentence that explains it, aligned with the control it acts for. */
  .fetch,
  .licence {
    grid-column: 2;
    justify-self: start;
    margin-top: 12px;
  }

  .models {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
  }

  /* The radio holds the first line, and the facts and credit hang under the
     name rather than under the radio. */
  .model {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 10px;
    padding: 6px 0;
    min-width: 0;
  }

  .model input {
    margin: 0;
    align-self: center;
  }

  .model label,
  .facts,
  .credit {
    grid-column: 2;
    min-width: 0;
  }

  .model label {
    cursor: pointer;
    overflow-wrap: anywhere;
  }

  .name {
    font-variation-settings:
      'wght' 600,
      'wdth' 100;
    font-size: 15px;
  }

  .on .name {
    font-variation-settings:
      'wght' 800,
      'wdth' 105;
  }

  /* The dash this world uses for a thing that is not here yet. */
  .absent {
    border-bottom: 1px dashed var(--accent);
  }

  .facts,
  .credit {
    margin: 2px 0 0;
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    line-height: 1.45;
  }

  .credit {
    color: var(--dim);
  }

  .credit button {
    margin-left: 4px;
    color: var(--ink);
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

  .more {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--ink);
    margin-bottom: 22px;
  }

  .more svg {
    transition: transform 120ms ease-out;
  }

  /* The chevron points where the rows went. */
  .more[aria-expanded='true'] svg {
    transform: rotate(180deg);
  }

  @media (prefers-reduced-motion: reduce) {
    .more svg {
      transition: none;
    }
  }
</style>
