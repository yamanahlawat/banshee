<script lang="ts">
  import { daemon, fixGroups, fixProse, type Blocker } from '../lib/daemon';
  import { announce } from '../lib/copy';
  import { downloadModels, openPermissionPane } from '../lib/tauri';
  import Command from '../controls/Command.svelte';
  import Filled from '../controls/Filled.svelte';

  // A dead daemon cannot act on anything the last status reported, so the
  // one thing it can still be told stands alone.
  $: blockers =
    $daemon.down !== null
      ? [{ kind: 'daemon', id: 'daemon', name: 'Banshee', consequence: 'nothing works until it runs', fix: 'start it: banshee start', command: 'banshee start' }]
      : ($daemon.status?.blockers ?? []);
  $: groups = fixGroups(blockers);
  $: permissions = blockers.filter((b) => b.kind === 'permission').length;
  $: models = blockers.filter((b) => b.kind === 'model').length;

  function plural(count: number, word: string): string {
    return `${count} ${count === 1 ? word : `${word}s`}`;
  }

  // The list is empty while a download runs, and when an empty history alone
  // holds the pad shut.
  function summaryFor(down: string | null, waiting: number, downloading: boolean): string {
    if (down !== null) return 'Banshee is not running.';
    if (waiting === 0) {
      return downloading
        ? 'Downloading what Banshee needs to hear you.'
        : 'Nothing left to fix. Say something and it lands here.';
    }
    if (permissions > 0) {
      return `${plural(permissions, 'permission')} to grant. Banshee restarts itself when each one lands.`;
    }
    if (models > 0) {
      return `${plural(models, 'model')} to download. Banshee uses them to hear you.`;
    }
    return 'One thing to fix before Banshee can hear you.';
  }

  $: summary = summaryFor($daemon.down, groups.length, $daemon.downloading);

  async function act(blocker: Blocker) {
    try {
      await (blocker.kind === 'model' ? downloadModels() : openPermissionPane(blocker.id));
    } catch (error) {
      announce((error as { message?: string })?.message || 'That did not work');
    }
  }

  // Nothing in the window can spawn the daemon, so a row that cannot act
  // offers the command alone.
  const ACTIONABLE = ['permission', 'model'];
</script>

<section aria-label="Setup" style="padding: 18px 22px 12px; display: flex; flex-direction: column; gap: 2px; flex: 1;">
  <span class="caps" style="color: var(--dim); margin-bottom: 8px;">Setup</span>
  <p style="margin: 0 0 10px;">{summary}</p>
  <ol style="margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column;">
    {#each groups as group, i (group[0].id)}
      {@const blocker = group[0]}
      {@const prose = fixProse(blocker)}
      <li style="display: flex; flex-direction: column; gap: 6px; padding: 10px 0; border-top: {i ? '1px solid var(--rule)' : '0'}; min-width: 0;">
        <span style="font-weight: 600;">{group.map((b) => b.name).join(', ')}</span>
        <p style="margin: 0;">Without {group.length > 1 ? 'them' : 'it'}, {blocker.consequence}.</p>
        {#if ACTIONABLE.includes(blocker.kind)}
        <div style="display: flex; align-items: center; gap: 12px;">
          <Filled label={blocker.kind === 'model' ? 'Download models' : `Open ${blocker.name} settings`} press={() => act(blocker)} />
          {#if blocker.kind === 'permission'}
            <span style="color: var(--dim);">Turn on Banshee in the list that opens.</span>
          {/if}
        </div>
        {/if}
        {#if prose}
          <p style="margin: 0; color: var(--dim);">{prose}</p>
        {/if}
        {#if blocker.command}
          <Command text={blocker.command} id={`fix:${blocker.id}`} />
        {/if}
      </li>
    {/each}
  </ol>
  {#if groups.length > 0}
    <p style="margin: 12px 0 0; color: var(--dim);">Try it opens once the rows above are clear.</p>
  {/if}
</section>
