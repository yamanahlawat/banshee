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

<!-- The sentence is drawn, not hidden. A bar held the place instead, which said
     nothing to a reader and told the two modes apart by hue alone. A turn is
     genuinely on its way here, so this is a place kept rather than a presence
     invented. -->
<article class="turn">
  <span class="mono time" aria-hidden="true">{time}</span>
  <p class="text">{SAYS[mode]}</p>
</article>

<style>
  /* Banshee's own voice, so the agent cut, and dim because this is a readout
     and not a turn anyone said. */
  .text {
    margin: 0;
    max-width: 520px;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
    color: var(--dim);
  }
</style>
