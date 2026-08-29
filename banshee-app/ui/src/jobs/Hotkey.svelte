<script lang="ts">
  import { daemon } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { hotkeyFrom, humanize, isModifier } from '../lib/hotkey';
  import Row from '../controls/Row.svelte';
  import More from '../controls/More.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import Toggle from '../controls/Toggle.svelte';
  import Action from '../controls/Action.svelte';

  let recording = false;
  let refusal: string | null = null;
  let heldModifier: string | null = null;

  $: audio = ($daemon.status?.config?.audio ?? {}) as Record<string, unknown>;
  $: key = String(audio.hotkey ?? '');
  $: mode = String(audio.hotkey_mode ?? 'hold');
  $: bargeIn = String(audio.barge_in ?? 'stop');
  $: cues = ((audio.cues ?? {}) as Record<string, unknown>).enabled === true;

  function stop() {
    recording = false;
    heldModifier = null;
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
    stop();
    refusal = null;
    write('audio.hotkey', next, (message) => (refusal = message));
  }

  function onKeyUp(event: KeyboardEvent) {
    // Only the release of the very modifier being held decides the binding.
    if (!recording || heldModifier === null || hotkeyFrom(event) !== heldModifier) return;
    event.preventDefault();
    const chosen = heldModifier;
    stop();
    refusal = null;
    write('audio.hotkey', chosen, (message) => (refusal = message));
  }
</script>

<svelte:window on:keydown={onKeyDown} on:keyup={onKeyUp} />

<Row name="Key" command={`banshee config set audio.hotkey ${key}`} pending={$daemon.pending.has('audio.hotkey')}>
  {#if recording}
    <span role="status" style="min-height: 30px; display: inline-flex; align-items: center;">Recording, press a key. Escape cancels.</span>
  {:else}
    <span class="mono" style="font-size: 13px; white-space: nowrap; min-height: 30px; display: inline-flex; align-items: center; padding: 0 10px; border: 1.5px solid var(--ink); border-radius: 6px; background: var(--field);">{humanize(key)}</span>
    <Action label="Change key" press={() => { recording = true; refusal = null; heldModifier = null; }} />
  {/if}
</Row>
<p style="margin: 0 0 4px 138px; color: var(--dim); font-size: 12.5px;">Change key records the next key you press. Escape cancels.</p>
{#if refusal}
  <p role="alert" style="margin: 0 0 4px 138px; color: var(--ink); font-size: 12.5px;">{refusal}</p>
{/if}

<Row name="Mode" command={`banshee config set audio.hotkey_mode ${mode}`} pending={$daemon.pending.has('audio.hotkey_mode')}>
  <Segmented
    label="Hotkey mode"
    active={mode}
    options={[{ value: 'hold', label: 'Hold' }, { value: 'toggle', label: 'Toggle' }]}
    change={(next) => write('audio.hotkey_mode', next, (message) => (refusal = message))}
  />
</Row>

<More />

<Row name="If you speak while it speaks" command={`banshee config set audio.barge_in ${bargeIn}`} pending={$daemon.pending.has('audio.barge_in')}>
  <Segmented
    label="When you speak while Banshee speaks"
    active={bargeIn}
    options={[{ value: 'stop', label: 'Stop' }, { value: 'none', label: 'Ignore' }]}
    change={(next) => write('audio.barge_in', next, (message) => (refusal = message))}
  />
</Row>

<Row name="Sounds" command={`banshee config set audio.cues.enabled ${cues}`} pending={$daemon.pending.has('audio.cues.enabled')}>
  <Toggle on={cues} label="Sounds on start, stop, ready and error" change={(next) => write('audio.cues.enabled', next, (message) => (refusal = message))} />
  <span>On start, stop, ready and error</span>
</Row>
