<script lang="ts">
  import { onDestroy } from 'svelte';
  import { daemon, deviceLabel, shownFloat, SYSTEM_DEVICE } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { listDevices, type Devices } from '../lib/tauri';
  import Field from '../controls/Field.svelte';
  import Picker from '../controls/Picker.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import { claimKeys } from '../lib/keys';

  // The word beside the slider, so the number is never the only reading.
  const BANDS = [
    { upTo: 1 / 3, word: 'Low' },
    { upTo: 2 / 3, word: 'Medium' },
    { upTo: Infinity, word: 'High' },
  ];
  const QUIET = [1000, 1500, 2500, 4000];
  const PRESETS = [
    { value: 'fast', label: 'Fast' },
    { value: 'balanced', label: 'Balanced' },
    { value: 'quality', label: 'Quality' },
  ];

  let devices: Devices = { devices: [], current: null };
  let adding = false;
  let field: HTMLInputElement | undefined;
  // Typing a word owns the keyboard: without this Escape closed the whole panel
  // instead of abandoning the word, and Cmd+F opened Find mid-word.
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
  $: vocabulary = (stt.vocabulary ?? []) as string[];
  $: preset = String(stt.preset ?? 'balanced');
  // `endpoint_silence_ms` is a plain u64 in the daemon, so a hand-edited config
  // can hold a value none of these offer.
  $: offered = QUIET.includes(silence) ? QUIET : [silence, ...QUIET];
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

<Field name="Input" pending={$daemon.pending.has('audio.input_device')}>
  <Picker
    label="Input device"
    value={current}
    change={(next) => write('audio.input_device', next)}
  >
    <option value={SYSTEM_DEVICE}>{deviceLabel($daemon.live.audio_device)}</option>
    {#each options as name (name)}
      <option value={name}>{name}</option>
    {/each}
  </Picker>
</Field>

<Field name="Sensitivity" pending={$daemon.pending.has('stt.vad_threshold')}>
  <input
    class="range"
    type="range"
    aria-label="Sensitivity"
    min="0"
    max="1"
    step="0.05"
    value={threshold}
    on:change={(e) => write('stt.vad_threshold', Number(e.currentTarget.value))}
  />
  <span class="readout">{band}</span>
</Field>

<Field name="End of speech" pending={$daemon.pending.has('stt.endpoint_silence_ms')}>
  <Picker
    label="End of speech"
    value={String(silence)}
    change={(next) => write('stt.endpoint_silence_ms', Number(next))}
  >
    {#each offered as ms (ms)}
      <option value={String(ms)}>After {ms / 1000} seconds of quiet</option>
    {/each}
  </Picker>
</Field>

<Field name="Transcription" pending={$daemon.pending.has('stt.preset')}>
  <Segmented
    label="Transcription"
    value={preset}
    options={PRESETS}
    change={(next) => write('stt.preset', next)}
  />
</Field>

<Field
  name="Vocabulary"
  note="Words Banshee should expect to hear."
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
          if (e.key === 'Escape') stopAdding();
        }}
      />
    {:else}
      <button class="btn btn-ghost" on:click={beginAdding}>Add a word</button>
    {/if}
  </div>
</Field>

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    width: 100%;
  }

  .add {
    background: transparent;
    width: 110px;
  }
</style>
