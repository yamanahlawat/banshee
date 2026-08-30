<script lang="ts">
  import { copied, copy } from '../lib/copy';

  export let text: string;
  export let time: string;
  export let speaker: 'user' | 'agent' = 'user';
  export let lead = false;
  export let id = '';

  // The cut carries the speaker only for anyone looking, so it is said in
  // words too.
  const SAID = { user: 'You said', agent: 'Banshee said' };
</script>

<article class="turn" class:lead data-speaker={speaker}>
  <span class="mono time" aria-hidden="true">{time}</span>

  <div class="col">
    <!-- Floated, not given a column: a column would charge 68px to every line
         of every turn for a control showing on one of them. -->
    {#if id}
      <button class="copy" on:click={() => copy(text, id)}>
        {$copied === id ? 'Copied' : 'Copy'}
        <span class="sr">{speaker === 'agent' ? 'what Banshee said' : 'what you said'} at {time}</span>
      </button>
    {/if}
    <p class="text">
      <span class="sr">{SAID[speaker]} at {time}.</span>{text}
    </p>
  </div>
</article>

<style>
  .turn {
    display: grid;
    grid-template-columns: 52px 1fr;
    column-gap: 12px;
    padding: 0 var(--gutter);
    margin-bottom: 20px;
  }

  .col {
    min-width: 0;
  }

  .time {
    font-size: 11px;
    line-height: 1.6;
    color: var(--accent);
    padding-top: 3px;
  }

  .text {
    margin: 0;
    text-wrap: pretty;
    min-width: 0;
    /* ~62 characters at this size, in px because `ch` resolves against this
       font's own zero, and Archivo at wdth 112 makes that about 14px. */
    max-width: 520px;
  }

  [data-speaker='user'] .text {
    font-variation-settings: 'wght' var(--cut-user-weight), 'wdth' var(--cut-user-width);
    font-size: 15px;
    line-height: 1.4;
    letter-spacing: -0.01em;
  }

  [data-speaker='agent'] .text {
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
    letter-spacing: 0;
  }

  .lead {
    margin-bottom: 28px;
  }

  .lead .text {
    font-size: 28px;
    line-height: 1.3;
    letter-spacing: -0.03em;
  }

  .lead .time {
    padding-top: 8px;
  }

  .copy {
    float: right;
    margin: 0 0 4px 14px;
    /* Fixed width for the longer of the two labels, so confirming a copy cannot
       widen the float and rewrap the paragraph around it. */
    width: 68px;
    text-align: center;
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--ink);
    background: transparent;
    border: 1px solid var(--ink);
    border-radius: 0;
    padding: 4px 4px;
    cursor: pointer;
    opacity: 0;
    pointer-events: none;
  }

  .turn:hover .copy,
  .lead .copy,
  .copy:focus-visible {
    opacity: 1;
    pointer-events: auto;
  }

  .copy:hover {
    background: var(--ink);
    color: var(--ground);
  }
</style>
