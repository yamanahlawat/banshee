<script lang="ts">
  // Nothing here moves. The daemon streams no level and no partial transcript,
  // so a waveform or a blinking caret would animate data that does not exist.
  export let mode: 'recording' | 'transcribing';
  export let time: string;

  const SAYS = {
    recording: 'Recording. What you say will appear here.',
    transcribing: 'Working out what you said.',
  } as const;
</script>

<article class="turn" data-mode={mode}>
  <span class="mono time" aria-hidden="true">{time}</span>
  <p class="text">
    <span class="sr">{SAYS[mode]}</span>
    <span class="caret" aria-hidden="true"></span>
  </p>
</article>

<style>
  .turn {
    display: grid;
    grid-template-columns: 52px 1fr;
    column-gap: 12px;
    padding: 0 var(--gutter);
    margin-bottom: 20px;
  }

  .time {
    font-size: 11px;
    line-height: 1.6;
    color: var(--accent);
    padding-top: 3px;
  }

  .text {
    margin: 0;
    font-size: 15px;
    line-height: 1.4;
  }

  .caret {
    display: inline-block;
    width: 3px;
    height: 0.82em;
    vertical-align: -0.08em;
    background: var(--accent);
  }

  [data-mode='transcribing'] .caret {
    background: var(--dim);
  }

</style>
