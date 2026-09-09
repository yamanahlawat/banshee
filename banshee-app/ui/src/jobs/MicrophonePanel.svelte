<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { daemon, deviceLabel, shownFloat, waitsOnARestart, SYSTEM_DEVICE } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { PRESETS } from '../lib/presets';
  import { listDevices, listLanguages, type Devices, type Languages } from '../lib/tauri';
  import Row from '../controls/Row.svelte';
  import Field from '../controls/Field.svelte';
  import Picker from '../controls/Picker.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import { claimKeys } from '../lib/keys';
  import { announce } from '../lib/copy';

  // The choice and its consequence are one reading, so the group names the
  // sentence its radiogroup is described by.
  const LISTENER_NOTE = 'listener-note';
  // The whole group waits on one restart, so the group states it once.
  const TAKES_EFFECT = 'Your choice takes effect when Banshee restarts.';

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
  $: keyPresent = $daemon.status?.remote?.stt?.key_present === true;
  // The daemon says which listener is in force; `provider` above says only
  // which one was asked for. One sentence reads both, so the group cannot say
  // two things at once.
  $: inForceRemote = $daemon.status?.remote?.stt?.remote === true;
  // Empty, not null, when the address the listener runs on is no URL, so the
  // sentence cannot be left with a hole in it.
  $: goesTo = $daemon.status?.remote?.stt?.host || 'a remote server';
  $: pendingProvider = $waitsOnARestart.has('stt.provider');
  $: listenerNote = pendingProvider
    ? inForceRemote
      ? `Audio still goes to ${goesTo}. ${TAKES_EFFECT}`
      : `Audio still stays on this machine. ${TAKES_EFFECT}`
    : inForceRemote
      ? `Audio goes to ${goesTo}.`
      : keyPresent
        ? 'Audio stays on this machine. The server and key you set are still saved.'
        : 'Audio stays on this machine.';
  // The daemon sends the vocabulary as the remote request's prompt, so under a
  // remote listener these words leave the machine too.
  $: vocabularyNote = inForceRemote
    ? 'Words Banshee should expect to hear. They go to the server with your audio.'
    : 'Words Banshee should expect to hear.';
  $: lastError = $daemon.live.last_error;
  $: failureSays = lastError ? `The last dictation failed: ${lastError}.` : '';

  // A failure arrives on a push, with no control moving and no reader
  // necessarily looking. Seeded from the first value the panel sees, so opening
  // on an old failure says nothing.
  let sawFailure: string | null | undefined;
  $: {
    if (sawFailure !== undefined && failureSays !== '' && lastError !== sawFailure) {
      announce(failureSays);
    }
    sawFailure = lastError;
  }

  // Three fields appear or leave with no event of their own, and where the
  // audio goes changes with them, so a reader who is not looking hears the
  // consequence before the layout. Seeded from the first status the panel sees,
  // because arriving on a remote listener is not a disclosure.
  let heard: string | null = null;
  $: if ($daemon.status) {
    if (heard !== null && heard !== provider) {
      announce(
        `${listenerNote} ${provider === 'remote' ? 'Server, model and key are below.' : 'Model is below.'}`,
      );
    }
    heard = provider;
  }
  $: language = String(stt.language ?? 'en');
  $: translate = stt.translate === true;
  // The daemon's own word, so the preset name is not a second rule for one fact.
  $: englishOnly = $daemon.status?.english_only === true;
  // A code Whisper's table does not name still needs its row: a select whose value matches no
  // option draws empty.
  $: languagesOffered =
    language === 'auto' || spoken.languages.some((one) => one.code === language)
      ? spoken.languages
      : [{ code: language, name: language }, ...spoken.languages];

  $: languageNote = !languagesArrived
    ? 'Banshee could not list the languages it knows. The one set here still applies.'
    : englishOnly
      ? 'Fast hears English only. Choose Balanced or Quality above to speak another language.'
      : 'The language you speak. Naming it beats detecting it.';

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

  // The daemon reads an empty key as removal.
  async function removeKey() {
    if (await write('stt.remote.api_key', '')) {
      announce('The key is removed. It takes effect when Banshee restarts.');
    }
  }

  // The key it holds cannot be shown, so replacing one is a field that arrives
  // where the line saying it is set stood.
  let replacing = false;
  let keyBox: HTMLElement | undefined;
  async function replaceKey() {
    replacing = true;
    await tick();
    keyBox?.querySelector('input')?.focus();
  }

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

<!-- One decision with dependents, so one group: the parts sit 8px apart inside
     it, and the 22px under the row above plus this padding is the 28px that
     holds the whole group away from the properties around it. -->
<div class="apart">
  <Row name="Listening" block note={listenerNote} noteId={LISTENER_NOTE}>
    <Segmented
      label="Listening"
      value={provider}
      options={LISTENING}
      describedBy={LISTENER_NOTE}
      change={(next) => write('stt.provider', next)}
    />

    <div class="parts" slot="under">
      {#if provider === 'remote'}
        <div class="part">
          <span class="sub mono">server</span>
          <div class="control">
            <Field
              label="Server"
              value={String(remoteTable.base_url ?? '')}
              placeholder="https://api.openai.com/v1"
              commit={(next) => write('stt.remote.base_url', next)}
            />
          </div>
        </div>
        <div class="part">
          <span class="sub mono">model</span>
          <div class="control">
            <Field
              label="Model"
              value={String(remoteTable.model ?? '')}
              placeholder="whisper-1"
              commit={(next) => write('stt.remote.model', next)}
            />
          </div>
        </div>
        <!-- Write-only. The daemon says whether a key is set and never what it is. -->
        <div class="part">
          <span class="sub mono">key</span>
          <div class="control" bind:this={keyBox}>
            {#if keyPresent && !replacing}
              <span class="held">A key is set</span>
              <button
                class="caps btn-underline act"
                aria-label="Remove the key"
                on:click={removeKey}
              >
                Remove
              </button>
              <button
                class="caps btn-underline act"
                aria-label="Replace the key"
                on:click={replaceKey}
              >
                Replace
              </button>
            {:else}
              <Field
                label="Key"
                masked
                dashed={!keyPresent}
                placeholder="Paste a key"
                cancel={() => (replacing = false)}
                commit={(next) => {
                  replacing = false;
                  return next === '' ? undefined : write('stt.remote.api_key', next);
                }}
              />
            {/if}
          </div>
        </div>
      {:else}
        <div class="part">
          <span class="sub mono">model</span>
          <div class="control">
            <Segmented
              label="Model"
              value={preset}
              options={PRESETS}
              change={(next) => write('stt.preset', next)}
            />
          </div>
        </div>
      {/if}

      <!-- Beside the key and the server that caused it, not at the head of the
           panel where the reader has already left the group. -->
      {#if failureSays}
        <p class="note failed">{failureSays}</p>
      {/if}
    </div>
  </Row>
</div>

<!-- Beside the preset it depends on: the English-only model rules every other
     language out, and the two read as one decision only if they sit together. -->
<Row name="Language" block note={languageNote} pending={$waitsOnARestart.has('stt.language')}>
  <Picker
    label="Language"
    value={language}
    disabled={englishOnly}
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

  /* Inside the group, so the interval is the 8px that says these parts belong
     to the choice above them rather than the 22px between properties. */
  .parts {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .part {
    display: grid;
    grid-template-columns: 56px 1fr;
    column-gap: 12px;
    align-items: baseline;
  }

  /* Subordinate to the one accent caps name the group carries, so the mono role
     without the caps register, and the measured dim rather than an opacity. */
  .sub {
    font-size: 11px;
    color: var(--dim);
  }

  .apart {
    padding-top: 6px;
  }

  /* A resting rule, or an 11px word beside a field reads as part of the value. */
  .act {
    color: var(--ink);
    border-bottom-color: currentcolor;
  }

  /* The field's own box, so the row's baseline and its edge stay put when the
     line and the field swap places. */
  .held {
    font-variation-settings:
      'wght' 500,
      'wdth' 100;
    font-size: 15px;
    padding: 6px 0 7px;
  }

  /* Under the fields that caused it, not under the name of the group. */
  .failed {
    margin: 0 0 0 68px;
  }
</style>
