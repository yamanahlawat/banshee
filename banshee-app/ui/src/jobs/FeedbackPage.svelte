<script lang="ts">
  import { daemon } from '../lib/daemon';
  import { defaultOf, optionsOf, table, write } from '../lib/settings';
  import { onMac } from '../lib/keys';
  import Row from '../controls/Row.svelte';
  import Segmented from '../controls/Segmented.svelte';

  $: feedbackConfig = ($daemon.status?.config?.feedback ?? {}) as Record<string, unknown>;
  $: feedback = String(feedbackConfig.mode ?? defaultOf($table, 'feedback.mode'));
  // The menu bar icon draws the figure only on macOS, so only there does the
  // row offer On screen. Read once: the platform does not change under this window.
  const mac = onMac();
  $: FEEDBACK_OPTIONS = optionsOf($table, 'feedback.mode').filter(
    (option) => mac || option.value !== 'visual',
  );
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
</script>

<Row name="Feedback" note={feedbackNote} noteId={FEEDBACK_NOTE} setting="feedback.mode">
  <Segmented
    label="Feedback"
    value={feedbackSelected}
    options={FEEDBACK_OPTIONS}
    describedBy={FEEDBACK_NOTE}
    change={(next) => write('feedback.mode', next)}
  />
</Row>
