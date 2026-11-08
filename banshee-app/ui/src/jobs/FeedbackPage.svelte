<script lang="ts">
  import { daemon } from '../lib/daemon';
  import { defaultOf, optionsOf, table, write } from '../lib/settings';
  import { onMac } from '../lib/keys';
  import Row from '../controls/Row.svelte';
  import Segmented from '../controls/Segmented.svelte';

  $: feedbackConfig = ($daemon.status?.config?.feedback ?? {}) as Record<string, unknown>;
  $: feedback = String(feedbackConfig.mode ?? defaultOf($table, 'feedback.mode'));
  $: FEEDBACK_OPTIONS = optionsOf($table, 'feedback.mode');
  const FEEDBACK_NOTE = 'feedback-note';
  // The page's lead says what shows and what plays, so a note says only the rest.
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
  $: feedbackNote = FEEDBACK_NOTES[feedback] ?? '';
</script>

<Row name="Feedback" note={feedbackNote} noteId={FEEDBACK_NOTE} setting="feedback.mode">
  <Segmented
    label="Feedback"
    value={feedback}
    options={FEEDBACK_OPTIONS}
    describedBy={FEEDBACK_NOTE}
    change={(next) => write('feedback.mode', next)}
  />
</Row>
