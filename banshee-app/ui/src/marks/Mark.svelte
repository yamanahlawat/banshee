<script lang="ts">
  import type { LampForm } from '../lib/daemon';

  export let form: LampForm = 'idle';
  export let size = 34;

  // Copied from `assets/banshee-mark.svg`. The menu bar draws that file, so the
  // two must stay one silhouette.
  const SHROUD =
    'M21 70 L24 46 C24 27 34 14 50 14 C66 14 76 27 76 46 L79 70 C79 80 72 88 64 86 ' +
    'C57 84 55 72 50 72 C45 72 43 84 36 86 C28 88 21 80 21 70 Z';

  // The busy ring: an outer ellipse minus an inner one shifted up, so the band
  // tapers instead of holding one width all the way round. A stroke cannot do
  // that - two strokes of different widths cannot join without a visible step.
  const RING =
    'M6 40 a44 18 0 1 0 88 0 a44 18 0 1 0 -88 0 z ' +
    'M9.5 38.8 a40 14.2 0 1 0 80 0 a40 14.2 0 1 0 -80 0 z';

  // The menu bar renders this monochrome, so shape alone tells the states apart.
  $: filled = form === 'recording';
</script>

<!-- Svelte restarts a CSS animation only on a fresh node, so the settle needs
     the key to replay. -->
{#key form}
  <svg
    class="mark"
    width={size}
    height={size}
    viewBox="0 0 100 100"
    aria-hidden="true"
    focusable="false"
  >
    {#if filled}
      <path
        d={SHROUD}
        fill="var(--accent)"
        stroke="var(--accent)"
        stroke-width="9"
        stroke-linejoin="round"
      />
    {:else}
      <path
        d={SHROUD}
        fill="none"
        stroke="currentColor"
        stroke-width="9"
        stroke-linejoin="round"
        stroke-dasharray={form === 'notrunning' ? '22 14' : undefined}
      />
      {#if form === 'listening'}
        <!-- Solid over-ear headphones. The window draws this mark at 34px.
             The cups must not resemble recording, the only other form that
             fills the whole body solid. -->
        <ellipse cx="17" cy="46" rx="12" ry="17" fill="currentColor" />
        <ellipse cx="83" cy="46" rx="12" ry="17" fill="currentColor" />
      {/if}
      {#if form === 'busy'}
        <!-- A ring behind the head: masked so it is hidden wherever the
             shroud is, then drawn again clipped to its lower half, so that
             near half crosses in front of the head instead of behind it. -->
        <mask id="mark-busy-mask">
          <rect x="-40" y="-40" width="180" height="180" fill="#fff" />
          <path d={SHROUD} fill="#000" stroke="#000" stroke-width="15" stroke-linejoin="round" />
        </mask>
        <clipPath id="mark-busy-near">
          <rect x="-40" y="40" width="180" height="100" />
        </clipPath>
        <path d={RING} fill="currentColor" fill-rule="evenodd" mask="url(#mark-busy-mask)" />
        <path d={RING} fill="currentColor" fill-rule="evenodd" clip-path="url(#mark-busy-near)" />
      {/if}
      {#if form === 'speaking'}
        <!-- The gap is the point: closed up, the arcs read as earmuffs rather
             than as sound leaving the figure. The menu bar draws them 5 units
             tighter because 36px cannot hold this pair, and that is the one
             place the two surfaces differ. -->
        <path
          d="M8 40 C4 48 4 56 8 64"
          fill="none"
          stroke="currentColor"
          stroke-width="6"
          stroke-linecap="round"
        />
        <path
          d="M92 40 C96 48 96 56 92 64"
          fill="none"
          stroke="currentColor"
          stroke-width="6"
          stroke-linecap="round"
        />
      {/if}
    {/if}
  </svg>
{/key}

<style>
  .mark {
    display: block;
    animation: settle 260ms cubic-bezier(0.16, 1, 0.3, 1);
    transform-origin: 50% 62%;
  }

  @keyframes settle {
    from {
      transform: scale(0.9);
      opacity: 0.35;
    }
    to {
      transform: scale(1);
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .mark {
      animation: none;
    }
  }
</style>
