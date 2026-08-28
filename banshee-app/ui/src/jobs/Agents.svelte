<script lang="ts">
  import { onMount } from 'svelte';
  import { applyConnect, planConnect, type AgentRow, type PlannedChange } from '../lib/tauri';
  import { agents, refresh } from '../lib/agents';
  import { showCommands } from '../lib/settings';
  import Action from '../controls/Action.svelte';
  import Filled from '../controls/Filled.svelte';
  import Command from '../controls/Command.svelte';

  let reviewing: { agent: AgentRow; plan: PlannedChange[] } | null = null;
  let rowErrors: Record<string, string> = {};

  onMount(refresh);

  // The daemon can only add banshee to an agent's config today, never
  // remove it, so a review always plans a connect.
  async function review(agent: AgentRow) {
    rowErrors = { ...rowErrors, [agent.id]: '' };
    try {
      const changes = await planConnect(agent.id, false);
      // An empty plan has nothing to show a review for; the row's own state
      // already says whether it is connected.
      if (changes.length === 0) return;
      reviewing = { agent, plan: changes };
    } catch (error) {
      rowErrors = { ...rowErrors, [agent.id]: (error as { message?: string })?.message || 'That failed.' };
    }
  }

  function dismiss() {
    reviewing = null;
  }

  // The rows say what is connected, so the review closes on the fresh list
  // rather than on the one the apply has already made wrong.
  async function apply() {
    if (reviewing === null) return;
    const id = reviewing.agent.id;
    try {
      await applyConnect(id, false);
    } catch (error) {
      rowErrors = { ...rowErrors, [id]: (error as { message?: string })?.message || 'That failed.' };
    }
    await refresh();
    dismiss();
  }
</script>

<p style="margin: 6px 0 4px; color: var(--dim);">Coding agents found on this Mac. Connecting one lets it hear you and speak back.</p>
<ul style="margin: 0; padding: 0; list-style: none;">
  {#each $agents as agent (agent.id)}
    <li style="display: flex; flex-direction: column; gap: 4px; padding: 4px 0; border-top: 1px solid var(--rule);">
      <div style="display: flex; align-items: center; gap: 12px; min-height: 40px;">
        <span style="width: 96px; font-weight: 600; flex-shrink: 0;">{agent.name}</span>
        {#if rowErrors[agent.id]}
          <span role="alert" style="flex: 1; color: var(--dim); min-width: 0;">{rowErrors[agent.id]}</span>
        {:else}
          <span style="flex: 1; color: var(--dim); min-width: 0;">{agent.note}</span>
        {/if}
        {#if reviewing?.agent.id === agent.id}
          <span class="caps" style="color: var(--dim);">Reviewing</span>
        {:else if agent.presence === 'found'}
          <Action label="Connect" press={() => review(agent)} />
        {/if}
      </div>
      {#if $showCommands && agent.presence === 'found'}
        <Command text={`banshee connect ${agent.id}`} id={`command:${agent.id}`} />
      {/if}
    </li>
  {/each}
</ul>

{#if reviewing !== null}
  {#each reviewing.plan as change}
    {#if change.path}
      <p style="margin: 12px 0 6px;"><strong>{reviewing.agent.name}</strong> would get this change to <span class="mono" style="font-size: 12px;">{change.path}</span>. Nothing is written until you apply it.</p>
    {:else}
      <p style="margin: 12px 0 6px;"><strong>{reviewing.agent.name}</strong> would run this command. Nothing is written until you apply it.</p>
    {/if}
    <pre class="mono" style="margin: 0 0 10px; padding: 10px 12px; border-left: 2px solid var(--ink); background: var(--strip); line-height: 1.5; overflow: auto;">{change.diff}</pre>
  {/each}
  <div style="display: flex; gap: 10px; align-items: center; margin-top: 4px;">
    <Filled label="Apply" press={apply} />
    <Action label="Not now" press={dismiss} />
  </div>
{/if}
