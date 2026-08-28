<script lang="ts">
  import { daemon, lampForm, stateWord } from '../lib/daemon';
  import Lamp from '../controls/Lamp.svelte';
  import { humanize } from '../lib/hotkey';

  $: word = stateWord($daemon);
  $: form = lampForm(word);
  $: audio = $daemon.status?.config?.audio as { hotkey?: string; hotkey_mode?: string } | undefined;
  // Before the daemon answers there is nothing to report, and a sentence with
  // a gap where the key belongs reads as a missing word rather than a wait.
  // A key the daemon has not bound yet does nothing, so the sentence says
  // that instead of naming a key the user would press in vain.
  function hotkeyInstruction(config: typeof audio, key: string, waiting: boolean): string {
    if (config === undefined) return '';
    if (key === '') return 'No hotkey set';
    if (waiting) return `Restart Banshee to use ${key}`;
    return config.hotkey_mode === 'hold' ? `Hold ${key} to talk` : `Press ${key} to start and stop`;
  }

  $: key = humanize(String(audio?.hotkey ?? ''));
  $: instruction = hotkeyInstruction(audio, key, $daemon.pending.has('audio.hotkey'));
</script>

<header style="display: flex; flex-direction: column; gap: 10px; padding: 14px 22px 12px; border-bottom: 1px solid var(--rule);">
  <div style="display: flex; align-items: center; justify-content: space-between; gap: 12px;">
    <div style="display: flex; align-items: center; gap: 9px;">
      <Lamp {form} />
      <span style="font-size: 16px; font-weight: 600; letter-spacing: -0.01em;">{word}</span>
    </div>
    <span style="font-size: 12px; color: var(--dim); text-align: right;">{instruction}</span>
  </div>
</header>
