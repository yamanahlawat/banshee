<script lang="ts">
  import type { LampForm } from '../lib/daemon';

  export let form: LampForm = 'idle';
  export let size = 34;

  // Copied from `assets/banshee-mark.svg`. The menu bar draws that file, so the
  // two must stay one silhouette.
  const SHROUD =
    'M21 70 L24 46 C24 27 34 14 50 14 C66 14 76 27 76 46 L79 70 C79 80 72 88 64 86 ' +
    'C57 84 55 72 50 72 C45 72 43 84 36 86 C28 88 21 80 21 70 Z';

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
        <!-- One bar, low in the hood. It has to hold at 18px against four
             other forms, and must not resemble recording, which is the only
             other form carrying solid ink. -->
        <rect x="33" y="52" width="34" height="10" fill="currentColor" />
      {/if}
      {#if form === 'speaking'}
        <path
          d="M8 40 C4 48 4 56 8 64"
          fill="none"
          stroke="currentColor"
          stroke-width="8"
          stroke-linecap="round"
        />
        <path
          d="M92 40 C96 48 96 56 92 64"
          fill="none"
          stroke="currentColor"
          stroke-width="8"
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
