<script lang="ts">
  import { daemon, deviceLabel, shownFloat, SYSTEM_DEVICE } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { listDevices, type Devices } from '../lib/tauri';
  import Row from '../controls/Row.svelte';
  import More from '../controls/More.svelte';
  import Picker from '../controls/Picker.svelte';
  import Slider from '../controls/Slider.svelte';
  import Action from '../controls/Action.svelte';
  import Segmented from '../controls/Segmented.svelte';

  // The word beside the slider, so the number is never the only reading.
  const BANDS = [
    { upTo: 1 / 3, word: 'Low' },
    { upTo: 2 / 3, word: 'Medium' },
    { upTo: Infinity, word: 'High' },
  ];
  const QUIET = [1000, 1500, 2500, 4000];

  let devices: Devices = { devices: [], current: null };
  let seenOpen: string | null | undefined;
  let seenMissing: string | null | undefined;
  let adding = false;
  let field: HTMLInputElement | undefined;

  // A word already in the list is not a second entry, and an empty one is
  // nothing at all.
  function addWord(raw: string) {
    adding = false;
    const word = raw.trim();
    if (word === '' || vocabulary.includes(word)) return;
    write('stt.vocabulary', [...vocabulary, word]);
  }

  $: if (adding && field) field.focus();

  $: stt = ($daemon.status?.config?.stt ?? {}) as Record<string, unknown>;
  $: threshold = shownFloat(Number(stt.vad_threshold ?? 0.5));
  $: band = BANDS.find((b) => threshold < b.upTo)?.word ?? 'High';
  $: silence = Number(stt.endpoint_silence_ms ?? 2500);
  $: vocabulary = (stt.vocabulary ?? []) as string[];
  // The config is what a write changes, so it leads. The daemon's live device
  // answers before the first write and whenever capture rebinds on its own.
  $: current = String(
    ($daemon.status?.config?.audio?.input_device as string) ?? $daemon.live.audio_device ?? '',
  );
  $: choices = [
    { value: SYSTEM_DEVICE, label: deviceLabel($daemon.live.audio_device ?? null) },
    ...devices.devices.map((d) => ({ value: d.name, label: d.name })),
  ];

  // A device that is neither open nor awaited still needs the panel reopened:
  // nothing reports one arriving.
  $: followTheDevices($daemon.live.audio_device, $daemon.live.missing_device);

  async function followTheDevices(open: string | null, missing: string | null) {
    if (open === seenOpen && missing === seenMissing) return;
    seenOpen = open;
    seenMissing = missing;
    try {
      const read = await listDevices();
      // A second change can start while this one is in flight, and the older
      // read must not land on top of the newer one.
      if (open === seenOpen && missing === seenMissing) devices = read;
    } catch {
      devices = { devices: [], current: null };
    }
  }
</script>

<Row name="Input" command={`banshee config set audio.input_device "${current}"`} pending={$daemon.pending.has('audio.input_device')}>
  <Picker
    label="Input"
    value={current}
    options={choices}
    change={(next) => write('audio.input_device', next)}
  />
</Row>

<More />

<Row name="Sensitivity" command={`banshee config set stt.vad_threshold ${threshold}`} pending={$daemon.pending.has('stt.vad_threshold')}>
  <Slider label="Sensitivity" value={threshold} min={0} max={1} step={0.05} change={(next) => write('stt.vad_threshold', next)} />
  <span style="width: 56px; text-align: right;">{band}</span>
</Row>

<Row name="End of speech" command={`banshee config set stt.endpoint_silence_ms ${silence}`} pending={$daemon.pending.has('stt.endpoint_silence_ms')}>
  <Picker
    label="End of speech"
    wide={false}
    value={String(silence)}
    options={QUIET.map((ms) => ({ value: String(ms), label: `After ${ms / 1000} seconds of quiet` }))}
    change={(next) => write('stt.endpoint_silence_ms', Number(next))}
  />
</Row>

<Row name="Transcription" command={`banshee config set stt.preset ${String(stt.preset ?? 'balanced')}`} pending={$daemon.pending.has('stt.preset')}>
  <Segmented
    label="Transcription preset"
    active={String(stt.preset ?? 'balanced')}
    options={[{ value: 'fast', label: 'Fast' }, { value: 'balanced', label: 'Balanced' }, { value: 'quality', label: 'Quality' }]}
    change={(next) => write('stt.preset', next)}
  />
</Row>

<Row
  name="Vocabulary"
  command={`banshee config set stt.vocabulary "${vocabulary.join(',')}"`}
  pending={$daemon.pending.has('stt.vocabulary')}
>
  <div style="flex: 1; display: flex; flex-wrap: wrap; gap: 6px; align-items: center;">
    {#each vocabulary as word, i (`${word}-${i}`)}
      <span class="mono" style="padding: 3px 7px; border: 1px solid var(--ink); border-radius: 4px; display: inline-flex; align-items: center; gap: 6px;">
        {word}
        <button
          type="button"
          aria-label={`Remove ${word}`}
          onclick={() => write('stt.vocabulary', vocabulary.filter((_, at) => at !== i))}
          style="border: 0; background: transparent; color: var(--dim); padding: 0; width: 12px; height: 12px; line-height: 1; cursor: pointer; font: inherit;"
        >×</button>
      </span>
    {/each}
    {#if adding}
      <input
        aria-label="Add a word"
        bind:this={field}
        onkeydown={(event) => {
          if (event.key === 'Enter') addWord(event.currentTarget.value);
          if (event.key === 'Escape') adding = false;
        }}
        onblur={(event) => addWord(event.currentTarget.value)}
        style="font: inherit; min-height: 28px; padding: 0 7px; border: 1.5px solid var(--ink); border-radius: 4px; background: var(--field); color: var(--ink); min-width: 96px;"
      />
    {:else}
      <Action label="Add a word" press={() => { adding = true; }} />
    {/if}
  </div>
</Row>
<p style="margin: 2px 0 0 138px; color: var(--dim); font-size: 12.5px;">{vocabulary.length} {vocabulary.length === 1 ? 'word' : 'words'}.</p>
