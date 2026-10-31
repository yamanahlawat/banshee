<script lang="ts">
  // The window copies the daemon's command for a terminal and never runs it.
  import { copied, copy, licenceWait, readLicence } from '../lib/copy';
  import type { WaitsOnLicence } from '../lib/daemon';

  export let model: WaitsOnLicence;

  $: copyId = `accept-${model.id}`;
</script>

<div class="licence">
  <p class="note waits">{licenceWait(model)}</p>
  <p class="note">{model.licence.summary}</p>
  <p class="note">To accept it, run this in a terminal:</p>
  <code class="mono command">{model.accept_command}</code>
  <div class="links">
    <button
      class="caps btn-underline"
      aria-label={`Read the licence for ${model.label}`}
      on:click={() => readLicence(model)}>Read the licence</button
    >
    <button class="caps btn-underline" on:click={() => copy(model.accept_command, copyId)}>
      {$copied === copyId ? 'Copied' : 'Copy'}
      <span class="sr">the command</span>
    </button>
  </div>
</div>

<style>
  .licence {
    display: flex;
    flex-direction: column;
    align-items: start;
    gap: 6px;
  }

  .note {
    margin: 0;
  }

  .waits {
    color: var(--accent);
  }

  .command {
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .links {
    display: flex;
    gap: 16px;
    margin-top: 4px;
  }
</style>
