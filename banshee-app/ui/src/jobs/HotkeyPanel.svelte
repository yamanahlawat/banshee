<script lang="ts">
  import { onDestroy } from 'svelte';
  import { daemon, hotkeyListens } from '../lib/daemon';
  import { write } from '../lib/settings';
  import { hotkeyFrom, humanize, isModifier } from '../lib/hotkey';
  import { claimKeys, onMac } from '../lib/keys';
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
  $: mode = String(audio.hotkey_mode ?? 'hold');
  $: bargeIn = String(audio.barge_in ?? 'stop');
  // An older daemon sends no [feedback] table, so its cue switch decides.
  $: cues = (audio.cues ?? {}) as Record<string, unknown>;
  $: feedbackConfig = ($daemon.status?.config?.feedback ?? {}) as Record<string, unknown>;
  $: feedback = String(feedbackConfig.mode ?? (cues.enabled === false ? 'none' : 'both'));
  // The menu bar icon draws the figure only on macOS, so only there does the
  // row offer On screen. Read once: the platform does not change under this window.
  const mac = onMac();
  const FEEDBACK_BASE_OPTIONS = [
    { value: 'sound', label: 'Sound' },
    { value: 'both', label: 'Both' },
    { value: 'none', label: 'Off' },
  ];
  const FEEDBACK_OPTIONS = mac
    ? [{ value: 'visual', label: 'On screen' }, ...FEEDBACK_BASE_OPTIONS]
    : FEEDBACK_BASE_OPTIONS;
  // Off macOS the row has no On screen radio, so a daemon set to visual still needs a selection.
  $: visualWithoutFigure = feedback === 'visual' && !mac;
  $: feedbackSelected = visualWithoutFigure ? 'both' : feedback;
  const FEEDBACK_NOTE = 'feedback-note';
  const FEEDBACK_NOTES: Record<string, string> = {
    ...(mac && {
      visual:
        "A small Banshee figure above the Dock shows what Banshee does, with no sounds. An agent's questions are still spoken aloud. Using VoiceOver? Choose Both, so you hear when recording starts.",
    }),
    sound: 'A short sound when Banshee starts and stops listening, and when it fails.',
    both: mac ? 'The Banshee figure and every sound.' : 'Every sound.',
    none: mac
      ? "No figure and no sound. An agent's questions are still spoken aloud."
      : "No sound. An agent's questions are still spoken aloud.",
  };
  $: feedbackNote = visualWithoutFigure
    ? 'Set to visual outside this window. Without the on-screen figure, Banshee plays every sound.'
    : (FEEDBACK_NOTES[feedback] ?? '');

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
  <Row name="The key" pending={$daemon.pending.has('audio.hotkey')}>
    <button class="key" on:click={() => (recording ? stop() : begin())}>
      {recording ? 'Press a key' : humanize(key) || 'Not set'}
      <span class="sr">— change the hotkey</span>
    </button>
    <button class="btn" on:click={() => (recording ? stop() : begin())}>
      {recording ? 'Cancel' : 'Change'}
    </button>
  </Row>

  {#if refusal}
    <p class="refusal">{refusal}</p>
  {/if}

  <Row
    name="Press behaviour"
    note="Hold: speak while the key is down. Tap: press once to start, once to stop."
    pending={$daemon.pending.has('audio.hotkey_mode')}
  >
    <Segmented
      label="Press behaviour"
      value={mode}
      options={[
        { value: 'hold', label: 'Hold' },
        { value: 'toggle', label: 'Tap' },
      ]}
      change={(next) => write('audio.hotkey_mode', next)}
    />
  </Row>
{:else}
  <p class="compositor">
    Wayland grants no global hotkey, so Banshee binds none. Bind these two commands to one key in
    your compositor. Put the first on the press and the second on the release.
  </p>
  <pre class="commands">banshee record start --dictate
banshee record stop</pre>
  <p class="compositor">docs/linux.md gives the Hyprland and Omarchy syntax.</p>
{/if}

<Row name="While Banshee is talking" pending={$daemon.pending.has('audio.barge_in')}>
  <Segmented
    label="While Banshee is talking"
    value={bargeIn}
    options={[
      { value: 'stop', label: 'Stop' },
      { value: 'none', label: 'Carry on' },
    ]}
    change={(next) => write('audio.barge_in', next)}
  />
</Row>

<Row
  name="Feedback"
  note={feedbackNote}
  noteId={FEEDBACK_NOTE}
  pending={$daemon.pending.has('feedback.mode')}
>
  <Segmented
    label="Feedback"
    value={feedbackSelected}
    options={FEEDBACK_OPTIONS}
    describedBy={FEEDBACK_NOTE}
    change={(next) => write('feedback.mode', next)}
  />
</Row>

<style>
  .compositor {
    margin: 0 0 12px;
    color: var(--dim);
    font-size: 13px;
    line-height: 1.5;
  }

  .commands {
    margin: 0 0 12px;
    padding: 10px 12px;
    background: var(--foot);
    border-radius: 4px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--ink);
    overflow-x: auto;
  }

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
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    color: var(--accent);
  }
</style>
