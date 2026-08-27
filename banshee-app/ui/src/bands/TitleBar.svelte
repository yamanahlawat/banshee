<script lang="ts">
  import { daemon, lampForm, stateWord } from '../lib/daemon';
  import Lamp from '../controls/Lamp.svelte';

  // The daemon reports hotkeys with no spaces, e.g. RightCommand.
  function humanizeKey(key: string): string {
    return key.replace(/([a-z0-9])([A-Z])/g, '$1 $2');
  }

  $: word = stateWord($daemon);
  $: form = lampForm(word);
  $: audio = $daemon.status?.config?.audio as { hotkey?: string; hotkey_mode?: string } | undefined;
  // Before the daemon answers there is nothing to report, and a sentence with
  // a gap where the key belongs reads as a missing word rather than a wait.
  function hotkeyInstruction(config: typeof audio, key: string): string {
    if (config === undefined) return '';
    if (key === '') return 'No hotkey set';
    return config.hotkey_mode === 'hold' ? `Hold ${key} to talk` : `Press ${key} to start and stop`;
  }

  $: key = humanizeKey(String(audio?.hotkey ?? ''));
  $: instruction = hotkeyInstruction(audio, key);
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
