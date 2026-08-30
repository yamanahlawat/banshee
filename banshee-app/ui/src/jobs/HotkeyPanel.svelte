<script lang="ts">
  import { onDestroy } from 'svelte';
  import { daemon } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { hotkeyFrom, humanize, isModifier } from '../lib/hotkey';
  import { claimKeys } from '../lib/keys';
  import Field from '../controls/Field.svelte';
  import Segmented from '../controls/Segmented.svelte';

  let recording = false;
  let refusal: string | null = null;
  let heldModifier: string | null = null;
  // Held for as long as the capture runs, so Escape cancels the capture and a
  // chord containing Cmd or Ctrl binds rather than triggering the window.
  let release: (() => void) | null = null;

  function begin() {
    refusal = null;
    recording = true;
    release ??= claimKeys();
  }

  onDestroy(() => release?.());

  $: audio = ($daemon.status?.config?.audio ?? {}) as Record<string, unknown>;
  $: key = String(audio.hotkey ?? '');
  $: mode = String(audio.hotkey_mode ?? 'hold');
  $: bargeIn = String(audio.barge_in ?? 'stop');
  $: cues = ((audio.cues ?? {}) as Record<string, unknown>).enabled === true;

  function stop() {
    recording = false;
    heldModifier = null;
    release?.();
    release = null;
  }

  function commit(next: string) {
    stop();
    refusal = null;
    write('audio.hotkey', next);
  }

  // The window listens only while recording, so an ordinary press still
  // reaches whatever it would normally reach.
  function onKeyDown(event: KeyboardEvent) {
    if (!recording) return;
    event.preventDefault();
    if (event.key === 'Escape') {
      stop();
      return;
    }
    const next = hotkeyFrom(event);
    if (next === null) {
      refusal = 'Banshee cannot bind that key.';
      return;
    }
    // A chord begins with its modifiers, so committing on the first press
    // would bind the modifier and never see the key it was held for.
    if (isModifier(event.code)) {
      heldModifier = next;
      return;
    }
    commit(next);
  }

  // A lone modifier is a legal binding, and the release is the only moment
  // that tells it apart from the start of a chord.
  function onKeyUp(event: KeyboardEvent) {
    if (!recording || heldModifier === null) return;
    event.preventDefault();
    if (isModifier(event.code)) commit(heldModifier);
  }
</script>

<svelte:window on:keydown={onKeyDown} on:keyup={onKeyUp} />

<Field name="Hold to talk" pending={$daemon.pending.has('audio.hotkey')}>
  <button class="key" on:click={() => (recording ? stop() : begin())}>
    {recording ? 'Press a key' : humanize(key) || 'Not set'}
    <span class="sr">— change the hotkey</span>
  </button>
  <button class="btn" on:click={() => (recording ? stop() : begin())}>
    {recording ? 'Cancel' : 'Change'}
  </button>
</Field>

{#if refusal}
  <p class="refusal">{refusal}</p>
{/if}

<Field
  name="Press behaviour"
  note="Hold: speak while the key is down. Toggle: tap to start, tap to stop."
  pending={$daemon.pending.has('audio.hotkey_mode')}
>
  <Segmented
    label="Press behaviour"
    value={mode}
    options={[
      { value: 'hold', label: 'Hold' },
      { value: 'toggle', label: 'Toggle' },
    ]}
    change={(next) => write('audio.hotkey_mode', next)}
  />
</Field>

<Field
  name="While Banshee is talking"
  pending={$daemon.pending.has('audio.barge_in')}
>
  <Segmented
    label="While Banshee is talking"
    value={bargeIn}
    options={[
      { value: 'stop', label: 'Stop' },
      { value: 'duck', label: 'Quieten' },
      { value: 'none', label: 'Carry on' },
    ]}
    change={(next) => write('audio.barge_in', next)}
  />
</Field>

<Field name="Sounds" pending={$daemon.pending.has('audio.cues.enabled')}>
  <Segmented
    label="Sounds"
    value={cues ? 'on' : 'off'}
    options={[
      { value: 'on', label: 'On' },
      { value: 'off', label: 'Off' },
    ]}
    change={(next) => write('audio.cues.enabled', next === 'on')}
  />
</Field>

<style>
  /* A button, because this underline is the one the pickers wear and it has to
     mean the same thing on both: press here to change what it says. */
  .key {
    font-family: var(--mono);
    font-size: 13px;
    text-align: left;
    color: var(--ink);
    background: transparent;
    border: 0;
    border-bottom: 1px solid var(--ink);
    border-radius: 0;
    padding: 6px 0;
    flex: 1;
    cursor: pointer;
  }

  .key:hover {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }

  .refusal {
    margin: -16px 0 26px;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 13px;
    color: var(--accent);
  }
</style>
