<script lang="ts">
  import { onMount } from 'svelte';
  import { applyConnect, planConnect, type AgentRow, type PlannedChange } from '../lib/tauri';
  import { agents, refresh } from '../lib/agents';

  let reviewing: { agent: AgentRow; plan: PlannedChange[] } | null = null;
  let rowErrors: Record<string, string> = {};

  onMount(refresh);

  const SAYS: Record<string, string> = {
    connected: 'Connected',
    found: 'Installed',
  };

  // An agent that is not on this machine has no action and no state worth a row.
  $: here = $agents.filter((agent) => agent.presence !== 'absent');
  $: elsewhere = $agents.filter((agent) => agent.presence === 'absent').map((a) => a.name);
  $: alsoWorksWith =
    elsewhere.length === 0
      ? ''
      : elsewhere.length === 1
        ? elsewhere[0]
        : `${elsewhere.slice(0, -1).join(', ')} and ${elsewhere[elsewhere.length - 1]}`;

  // The daemon can only add banshee to an agent's config today, never remove
  // it, so a review always plans a connect.
  async function review(agent: AgentRow) {
    rowErrors = { ...rowErrors, [agent.id]: '' };
    try {
      const changes = await planConnect(agent.id, false);
      // An empty plan has nothing to show a review for; the row's own state
      // already says whether it is connected.
      if (changes.length === 0) return;
      reviewing = { agent, plan: changes };
    } catch (error) {
      rowErrors = {
        ...rowErrors,
        [agent.id]: (error as { message?: string })?.message || 'That failed.',
      };
    }
  }

  // The rows say what is connected, so the review closes on the fresh list
  // rather than on the one the apply has already made wrong.
  async function apply() {
    if (reviewing === null) return;
    const id = reviewing.agent.id;
    try {
      await applyConnect(id, false);
      await refresh();
      reviewing = null;
    } catch (error) {
      rowErrors = {
        ...rowErrors,
        [id]: (error as { message?: string })?.message || 'That failed.',
      };
      reviewing = null;
    }
  }
</script>

{#if reviewing}
  <div class="review">
    <span class="caps">What this changes</span>
    {#each reviewing.plan as change (change.diff)}
      {#if change.path}<p class="path mono">{change.path}</p>{/if}
      <pre class="diff">{change.diff}</pre>
    {/each}
    <div class="actions">
      <button class="btn" on:click={apply}>Apply</button>
      <button class="btn btn-ghost" on:click={() => (reviewing = null)}>Cancel</button>
    </div>
  </div>
{:else}
  <div class="rows">
    {#each here as agent (agent.id)}
      <div class="row">
        <span class="name">{agent.name}</span>
        <span class="presence caps" class:on={agent.presence === 'connected'}>
          {SAYS[agent.presence] ?? agent.presence}
        </span>
        {#if agent.presence === 'found'}
          <button class="btn" on:click={() => review(agent)}>Connect</button>
        {/if}
      </div>
      {#if rowErrors[agent.id]}<p class="error">{rowErrors[agent.id]}</p>{/if}
    {:else}
      <p class="lede">Looking for agents on this machine.</p>
    {/each}
  </div>

  {#if alsoWorksWith}
    <p class="elsewhere">Banshee also works with {alsoWorksWith}.</p>
  {/if}
{/if}

<style>
  .caps {
    color: var(--accent);
  }

  .lede {
    max-width: 520px;
    margin: 0 0 20px;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
  }

  .rows {
    display: flex;
    flex-direction: column;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 0;
    min-width: 0;
  }

  .name {
    font-variation-settings: 'wght' 700, 'wdth' 105;
    font-size: 15px;
    flex: 1;
    min-width: 0;
  }

  /* Both states are named here, because this row is the one place the caps type
     is not the accent by default: the accent is what connected means. */
  .presence {
    color: var(--ink);
  }

  .presence.on {
    color: var(--accent);
  }

  .elsewhere {
    max-width: 520px;
    margin: 16px 0 0;
    font-variation-settings: 'wght' var(--cut-agent-weight), 'wdth' var(--cut-agent-width);
    font-size: 13px;
    color: var(--dim);
  }

  .error {
    margin: 0 0 10px;
    font-size: 13px;
    color: var(--accent);
  }

  .path {
    margin: 12px 0 4px;
    font-size: 11px;
    color: var(--accent);
  }

  .diff {
    margin: 0;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-x: auto;
    border-left: 1px solid var(--rule);
    padding-left: 12px;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 18px;
  }
</style>
