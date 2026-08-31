<script lang="ts">
  import {
    downloadLine,
    percent,
    spokenProgress,
    fixGroups,
    fixProse,
    type Blocker,
    type Progress,
    type Remedy,
  } from '../lib/daemon';
  import { PRESETS, downloadSize } from '../lib/presets';
  import { write } from '../lib/settings';
  import { downloadModels, openPermissionPane } from '../lib/tauri';
  import { announce, spell } from '../lib/copy';
  import Segmented from '../controls/Segmented.svelte';

  export let blockers: Blocker[] = [];
  export let download: Progress | null = null;
  export let preset = 'balanced';
  /// What the daemon says this run still costs.
  export let megabytes = 0;
  /// The shell's, which re-reads the daemon afterwards. Restarting without that
  /// leaves this box asking for the restart it has just been given.
  export let restart: () => void;
  export let busy = false;
  /// Nothing has ever been dictated, so this is the one moment a person has the
  /// question "what is this". Derived, never stored: dictate once and it is
  /// answered for good.
  export let first = false;

  /// A grant reaches only a process started after it lands, so opening the pane
  /// cannot finish the job. The window cannot see the grant either, so after it
  /// has sent someone to System Settings it offers the restart that makes one
  /// real.
  let asked: Record<string, boolean> = {};


  $: groups = fixGroups(blockers);
  // What a reader can actually see, with its remedy resolved once. A group a
  // running download settles is not drawn, and the download box is, so neither
  // the count nor the numbering can be taken from `groups` alone.
  $: shown = groups
    .map((group) => ({ group, fix: decide(group) }))
    .filter(({ fix }) => !(download && fix.settledByADownload));
  // The download box holds the first number when there is one.
  $: offset = download ? 1 : 0;
  $: steps = shown.length + offset;

  /// The daemon names the remedy. `kind` says only which part is at fault, and
  /// the prose beside it is for reading, so neither can be routed on.
  /// A daemon older than `remedy` names only a command and a kind, so the
  /// reading it supports is stated once here rather than at each branch.
  function remedyOf(first: Blocker): Remedy | null {
    if (first.remedy) return first.remedy;

    if (first.kind === 'permission') return 'grant';
    if (first.command === 'banshee setup') return 'download';
    if (first.command === 'banshee start') return 'restart';
    return null;
  }

  function decide(group: Blocker[]) {
    const first = group[0];
    const microphone = first.kind === 'pipeline';
    switch (remedyOf(first)) {
      case 'grant':
        return {
          title: first.name,
          label: 'Open System Settings',
          // Every permission blocker draws this button. Heard one after the
          // other, the names are the same button twice.
          pane: first.name,
          // Opening the pane cannot finish this one: the grant reaches only a
          // process started afterwards.
          confirmsWithRestart: true,
          run: () => openPermissionPane(first.id),
          // The download brings no grant, so this keeps its place beside one.
          settledByADownload: false,
          chooses: false,
        };
      case 'download':
        return {
          title: 'Banshee needs its models',
          label: 'Download',
          pane: '',
          confirmsWithRestart: false,
          run: downloadModels,
          settledByADownload: true,
          // The preset decides which speech model is fetched, so it is worth
          // asking before a download that runs to gigabytes. A missing voice or
          // detector is the same whichever preset is set.
          chooses: group.some((blocker) => blocker.role === 'speech'),
        };
      case 'restart':
        return {
          // A microphone fault and a model fault share one remedy, and its own
          // fix says to connect the device first, so the restart is named as
          // the last resort it is.
          title: microphone ? 'The microphone is not working' : 'Banshee needs a restart',
          label: microphone ? 'Restart anyway' : 'Restart Banshee',
          pane: '',
          confirmsWithRestart: false,
          run: restart,
          settledByADownload: true,
          chooses: false,
        };
      default:
        return { title: first.name, label: null, pane: '', confirmsWithRestart: false, run: null, settledByADownload: false, chooses: false };
    }
  }

  /// A refused call says so rather than becoming an unhandled rejection: the
  /// daemon answers -32005 when a download is already running.
  function run(fix: { label: string | null; run: (() => unknown) | null }): () => void {
    return () => {
      if (!fix.run) return;
      Promise.resolve(fix.run()).catch(() => announce(`${fix.label} did not work.`));
    };
  }
</script>

<!-- A run fetches more than the daemon blocks on, so the two blocking files
     land first and the rest keep coming. Left to itself the box then states a
     restart and a download at once, as though they were one thing. Only the
     groups a download settles stand down: a permission is granted in System
     Settings whatever else is arriving. -->
{#if first && steps > 0}
  <!-- What Banshee is and what it still needs, as one statement. No welcome
       screen to dismiss: the window states what is true, and this stops being
       true once the first thing is said. -->
  <p class="opening">
    Banshee types what you say into whatever app you are using, and nothing you say leaves
    this machine. It needs {spell(steps)} {steps === 1 ? 'thing' : 'things'} first.
  </p>
{/if}

{#if download}
  {@const done = percent(download.bytes, download.total)}
  <section class="blocker">
    <h2 class="caps">{steps > 1 ? `1 of ${steps} · ` : ''}Getting Banshee's models</h2>
    <!-- The daemon streams this percent, so the bar draws a real value. It has
         no transition: the width moves because the number did, which is data
         and not an authored motion. -->
    {#if done !== null}
      <div class="track"><div class="bar" style="width: {done}%"></div></div>
    {/if}
    <p class="progress mono">{downloadLine(download)}</p>
    <div class="actions">
      <!-- No Try again here: a failed file does not end the run, and the daemon
           refuses a second one while the first holds the slot. A run that ends
           having failed clears this box and returns the blocker, whose own
           Download is the way back. -->
      <button class="btn" disabled>Downloading</button>
    </div>
    <!-- The daemon reports each percent, and a live region reads every change
         it is given, so what is said aloud steps in quarters instead. -->
    <span class="sr" aria-live="polite">{spokenProgress(download)}</span>
    <!-- No Cancel: the protocol has no method that stops a run. Adding one is
         daemon work, and a button the daemon cannot honour is worse than none. -->
  </section>
{/if}

{#each shown as { group, fix }, i (group[0].id)}
  {@const prose = fixProse(group[0])}
  <section class="blocker">
    <h2 class="caps">{steps > 1 ? `${i + 1 + offset} of ${steps} · ` : ''}{fix.title}</h2>
    <p class="consequence">Until this is done, {group[0].consequence}.</p>
    {#if prose}<p class="fix">{prose}</p>{/if}

    {#if fix.chooses}
      <div class="choice">
        <span class="caps label">Speech model</span>
        <Segmented
          label="Speech model"
          value={preset}
          options={PRESETS}
          change={(next) => write('stt.preset', next)}
        />
        <p class="size">
          About {downloadSize(megabytes)} to fetch. Faster models hear less well. You can
          change this later.
        </p>
      </div>
    {/if}

    {#if fix.label}
      <div class="actions">
        {#if fix.confirmsWithRestart && asked[group[0].id]}
          <button class="btn" on:click={restart} disabled={busy}>
            I granted it. Restart Banshee
          </button>
          <button class="btn btn-ghost" on:click={run(fix)} disabled={busy}>
            Open it again
            <span class="sr">for {fix.pane}</span>
          </button>
        {:else}
          <button
            class="btn"
            disabled={busy}
            on:click={() => {
              if (fix.confirmsWithRestart) asked = { ...asked, [group[0].id]: true };
              run(fix)();
            }}
          >
            {fix.label}
            {#if fix.pane}<span class="sr">for {fix.pane}</span>{/if}
          </button>
        {/if}
      </div>
    {/if}
    {#if fix.confirmsWithRestart && asked[group[0].id]}
    <p class="fix">A grant only reaches Banshee if it starts afterwards.</p>
    {/if}
  </section>
{/each}

<style>
  h2,
  .label {
    color: var(--accent);
  }

  h2 {
    margin: 0;
  }

  /* An absence is drawn, not omitted: the thing that is missing takes up the
     place it will fill once it arrives. */
  .blocker {
    margin: 0 var(--gutter) 16px;
    padding: 14px 16px;
    border: 1px dashed var(--accent);
  }

  .consequence,
  .fix,
  .size {
    max-width: 520px;
    margin: 8px 0 0;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
  }

  .fix {
    color: var(--accent);
  }

  .choice {
    margin-top: 16px;
  }

  .label {
    display: block;
    margin-bottom: 8px;
  }

  .size {
    margin-top: 10px;
    font-size: 13px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 14px;
  }

  .btn {
    /* Fits the longer label, so 'Downloading' cannot shove the command beside
       it sideways mid-download. */
    min-width: 132px;
  }

  .opening {
    max-width: 520px;
    margin: 0 var(--gutter) 22px;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
  }

  /* The world's own hairline, filled. No radius, no gradient, no glow. */
  .track {
    height: 1px;
    margin-top: 14px;
    background: var(--rule);
  }

  .bar {
    height: 1px;
    background: var(--accent);
  }

  .progress {
    margin: 10px 0 0;
    font-size: 11px;
    color: var(--accent);
  }
</style>
