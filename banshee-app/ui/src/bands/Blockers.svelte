<script lang="ts">
  import { fixGroups, fixProse, type Blocker } from '../lib/daemon';
  import { copied, copy } from '../lib/copy';
  import { downloadModels, openPermissionPane } from '../lib/tauri';

  export let blockers: Blocker[] = [];
  export let downloading = false;

  $: groups = fixGroups(blockers);

  // A group's first blocker names the fix; the rest of a model group are more
  // files behind the same one call.
  function title(group: Blocker[]): string {
    const [first] = group;
    if (first.kind === 'model') {
      return group.length === 1 ? 'One file is missing' : `${group.length} files are missing`;
    }
    return first.name;
  }

  async function act(group: Blocker[]) {
    const [first] = group;
    if (first.kind === 'model') {
      await downloadModels().catch(() => {});
      return;
    }
    if (first.kind === 'permission') {
      await openPermissionPane(first.id).catch(() => {});
    }
  }

  function actionLabel(group: Blocker[]): string | null {
    const [first] = group;
    if (first.kind === 'model') return downloading ? 'Downloading' : 'Download';
    if (first.kind === 'permission') return 'Open System Settings';
    return null;
  }
</script>

{#each groups as group (group[0].id)}
  {@const first = group[0]}
  {@const prose = fixProse(first)}
  {@const action = actionLabel(group)}
  <section class="blocker">
    <span class="caps">{title(group)}</span>
    <p class="consequence">Until this is done, {first.consequence}.</p>
    {#if prose}<p class="fix">{prose}</p>{/if}
    <div class="actions">
      {#if action}
        <button class="btn" on:click={() => act(group)} disabled={downloading && first.kind === 'model'}>
          {action}
        </button>
      {/if}
      {#if first.command}
        <button class="btn btn-ghost cmd" on:click={() => copy(first.command ?? '', first.id)}>
          {$copied === first.id ? 'Copied' : first.command}
        </button>
      {/if}
    </div>
  </section>
{/each}

<style>
  /* An absence is drawn, not omitted: the thing that is missing takes up the
     place it will fill once it arrives. */
  .blocker {
    margin: 0 var(--gutter) 16px;
    padding: 14px 16px;
    border: 1px dashed var(--accent);
  }

  .consequence,
  .fix {
    max-width: 520px;
    margin: 8px 0 0;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
  }

  .fix {
    color: var(--accent);
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

  .cmd {
    min-width: 0;
    text-transform: none;
    letter-spacing: 0;
  }
</style>
