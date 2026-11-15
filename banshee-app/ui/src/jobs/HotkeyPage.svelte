<script lang="ts">
  import { onDestroy } from 'svelte';
  import { daemon, hotkeyListens, isDown } from '../lib/daemon';
  import { defaultOf, forgetRefusal, optionsOf, refuse, table, write } from '../lib/settings';
  import { feedbackInForce } from '../lib/copy';
  import { hotkeyFrom, humanize, isModifier } from '../lib/hotkey';
  import { claimKeys, onMac } from '../lib/keys';
  import Row from '../controls/Row.svelte';
  import Segmented from '../controls/Segmented.svelte';

  let recording = false;
  let heldModifier: string | null = null;
  // Held for as long as the capture runs, so Escape cancels the capture and a
  // chord containing Cmd or Ctrl binds rather than triggering the window.
  let release: (() => void) | null = null;

  function begin() {
    forgetRefusal('audio.hotkey');
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
  $: shownKey = recording
    ? 'Press a key'
    : isDown($daemon)
      ? 'Unknown while Banshee is stopped'
      : humanize(key) || 'Not set';
  $: mode = String(audio.hotkey_mode ?? defaultOf($table, 'audio.hotkey_mode'));
  $: bargeInNote = `Stop: Banshee goes quiet when you ${listens ? 'press the key' : 'start dictating'}. Carry on: it finishes what it is saying.`;
  $: bargeIn = String(audio.barge_in ?? defaultOf($table, 'audio.barge_in'));

  $: feedbackConfig = ($daemon.status?.config?.feedback ?? {}) as Record<string, unknown>;
  $: feedback = String(feedbackConfig.mode ?? defaultOf($table, 'feedback.mode'));
  // The first sentence says what shows and what plays, so these say only the rest.
  const SPOKEN = "An agent's questions are still spoken aloud.";
  const FAILURE = 'A failure plays a sound too.';
  const mac = onMac();
  const FEEDBACK_NOTES: Record<string, string> = {
    visual: mac
      ? `${SPOKEN} Using VoiceOver? Choose Both, so you hear when recording starts.`
      : `${SPOKEN} The figure needs a desktop with layer shell, such as Hyprland, Sway or KDE. Without one, Banshee plays every sound.`,
    sound: FAILURE,
    both: FAILURE,
    none: SPOKEN,
  };
  // The daemon says which cues are in force; `feedback` above is only what was asked for.
  // A stopped Banshee shows and plays nothing, so the mode note would contradict it.
  $: feedbackNote = isDown($daemon)
    ? (feedbackInForce(null, false, mac) ?? '')
    : [feedbackInForce($daemon.live.feedback, true, mac), FEEDBACK_NOTES[feedback]]
        .filter(Boolean)
        .join(' ');
  function stop() {
    recording = false;
    heldModifier = null;
    release?.();
    release = null;
  }

  const toggle = () => (recording ? stop() : begin());

  function commit(next: string) {
    stop();
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
      const message =
        bindable.length === 0
          ? 'Banshee has to be running before a key can be bound.'
          : 'Banshee cannot bind that key.';
      refuse('audio.hotkey', message);
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
  <Row name="The key" setting="audio.hotkey" let:describedBy>
    <button class="key" class:waiting={recording} aria-describedby={describedBy} on:click={toggle}>
      {shownKey}<span class="sr">, change the hotkey</span>
    </button>
    <button class="btn holds" class:btn-ghost={recording} on:click={toggle}>
      <span>{recording ? 'Cancel' : 'Change'}</span>
      <!-- Holds the button at the wider of its two labels. -->
      <span class="held" aria-hidden="true">Change</span>
    </button>
  </Row>

  <Row
    name="Press behaviour"
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

<Row name="While Banshee is talking" note={bargeInNote} setting="audio.barge_in">
  <Segmented
    label="While Banshee is talking"
    value={bargeIn}
    options={optionsOf($table, 'audio.barge_in')}
    change={(next) => write('audio.barge_in', next)}
  />
</Row>

<Row name="Feedback" note={feedbackNote} setting="feedback.mode">
  <Segmented
    label="Feedback"
    value={feedback}
    options={optionsOf($table, 'feedback.mode')}
    change={(next) => write('feedback.mode', next)}
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
</style>
