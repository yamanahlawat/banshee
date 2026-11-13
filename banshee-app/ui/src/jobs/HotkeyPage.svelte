<script lang="ts">
  import { onDestroy } from 'svelte';
  import { daemon, hotkeyListens } from '../lib/daemon';
  import { defaultOf, optionsOf, table, write } from '../lib/settings';
  import { hotkeyFrom, humanize, isModifier } from '../lib/hotkey';
  import { claimKeys } from '../lib/keys';
  import Row from '../controls/Row.svelte';
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

  // The daemon binds no key on Wayland, so the capture control below would
  // write a setting nobody reads. The compositor holds the binding instead.
  $: listens = hotkeyListens($daemon);
  $: bindable = $daemon.status?.bindable_modifiers ?? [];
  $: audio = ($daemon.status?.config?.audio ?? {}) as Record<string, unknown>;
  $: key = String(audio.hotkey ?? '');
  $: mode = String(audio.hotkey_mode ?? defaultOf($table, 'audio.hotkey_mode'));
  $: bargeIn = String(audio.barge_in ?? defaultOf($table, 'audio.barge_in'));
  function stop() {
    recording = false;
    heldModifier = null;
    release?.();
    release = null;
  }

  const toggle = () => (recording ? stop() : begin());

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
    const next = hotkeyFrom(event, bindable);
    if (next === null) {
      // Which modifiers bind is the daemon's answer, and a daemon that has not
      // answered refuses all of them. Saying Banshee cannot bind a key it binds
      // every day sends the reader after the wrong thing.
      refusal =
        bindable.length === 0
          ? 'Banshee has to be running before a key can be bound.'
          : 'Banshee cannot bind that key.';
      return;
    }
    // A chord begins with its modifiers, so committing on the first press
    // would bind the modifier and never see the key it was held for.
    if (isModifier(event.code, bindable)) {
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
    if (isModifier(event.code, bindable)) commit(heldModifier);
  }
</script>

<svelte:window on:keydown={onKeyDown} on:keyup={onKeyUp} />

{#if listens}
  <Row name="The key" setting="audio.hotkey">
    <button class="key" class:waiting={recording} on:click={toggle}>
      {recording ? 'Press a key' : humanize(key) || 'Not set'}<span class="sr"
        >, change the hotkey</span
      >
    </button>
    <button class="btn holds" class:btn-ghost={recording} on:click={toggle}>
      <span>{recording ? 'Cancel' : 'Change'}</span>
      <!-- Holds the button at the wider of its two labels. -->
      <span class="held" aria-hidden="true">Change</span>
    </button>
  </Row>

  {#if refusal}
    <p class="note refusal">{refusal}</p>
  {/if}

  <Row
    name="Press behaviour"
    block
    note="Hold: speak while the key is down. Tap: press once to start, once to stop."
    setting="audio.hotkey_mode"
  >
    <Segmented
      label="Press behaviour"
      value={mode}
      options={optionsOf($table, 'audio.hotkey_mode')}
      change={(next) => write('audio.hotkey_mode', next)}
    />
  </Row>
{:else}
  <p class="note compositor">
    Wayland grants no global hotkey, so Banshee binds none. Bind these two commands to one key in
    your compositor. Put the first on the press and the second on the release.
  </p>
  <pre class="commands">banshee record start
banshee record stop</pre>
  <p class="note compositor">docs/linux.md gives the Hyprland and Omarchy syntax.</p>
{/if}

<Row name="While Banshee is talking" block setting="audio.barge_in">
  <Segmented
    label="While Banshee is talking"
    value={bargeIn}
    options={optionsOf($table, 'audio.barge_in')}
    change={(next) => write('audio.barge_in', next)}
  />
</Row>

<style>
  .compositor {
    margin: 0 0 12px;
    color: var(--dim);
  }

  .commands {
    margin: 0 0 12px;
    padding: 10px 12px;
    background: var(--foot);
    border-radius: var(--radius);
    font-family: var(--mono);
    font-size: 11px;
    color: var(--ink);
    overflow-x: auto;
  }

  .key {
    flex: 1;
    min-width: 0;
    min-height: var(--control-height);
    font-family: var(--mono);
    font-size: 13px;
    text-align: left;
    color: var(--ink);
    background: transparent;
    border: 1px solid var(--dim);
    border-radius: var(--radius);
    padding: 6px 9px;
    cursor: pointer;
  }

  .key:hover {
    color: var(--accent);
  }

  /* A dashed accent line is what this world draws around a thing that is not
     there yet: here, the key. */
  .key.waiting {
    border-style: dashed;
    border-color: var(--accent);
  }

  .holds {
    display: inline-grid;
  }

  .holds > * {
    grid-area: 1 / 1;
  }

  .held {
    visibility: hidden;
  }

  .refusal {
    margin: -16px 0 26px;
    color: var(--accent);
  }
</style>
