<script lang="ts">
  import { onMount } from 'svelte';
  import { land } from '../lib/focus';
  import { announce, whyItFailed } from '../lib/copy';
  import { applyConnect, planConnect, type AgentRow, type PlannedChange } from '../lib/tauri';
  import { agents, refresh } from '../lib/agents';
  import Row from '../controls/Row.svelte';
  import DiffReview from '../controls/DiffReview.svelte';
  import { leave, travel, watchGroups } from '../lib/travel';

  let reviewing: { agent: AgentRow; plan: PlannedChange[] } | null = null;
  let rowErrors: Record<string, string> = {};
  /// What is left to do after a connect, per agent, as the daemon said it.
  let rowNotes: Record<string, string> = {};
  /// What the list has to say about itself. `failed` separates a fault from a
  /// caveat, because the accent is the colour of something being wrong.
  let listNote: { text: string; failed: boolean } | null = null;
  /// Whether a read has answered at all. An empty list before the first answer
  /// is a page still looking; after one, it is a machine with no agents.
  let looked = false;

  let applyButton: HTMLButtonElement | undefined;
  let applying = false;
  const nameOf = (id: string) => document.getElementById(`agent-${id}`);
  const connectOf = (id: string) => document.getElementById(`connect-${id}`);

  onMount(look);

  /// Detection answering `false` is not an empty machine. A page that keeps
  /// saying it is looking would report a green check the daemon never gave.
  async function look() {
    listNote = null;
    const read = await refresh();
    looked = true;
    if (!read) {
      listNote = {
        text: 'Banshee could not read which agents are installed. Banshee is probably not running.',
        failed: true,
      };
    }
  }

  // An agent that changes group travels. One not on this machine has no row.
  const moved = watchGroups();
  $: {
    const key = moved(
      $agents.map((agent) => ({
        key: agent.id,
        group: agent.presence === 'absent' ? null : agent.presence,
      })),
    );
    if (key !== null) travel(key);
  }

  // An agent that is not on this machine has no action and no state worth a row.
  // The daemon lists agents alphabetically, and each group keeps its order.
  $: groups = [
    { name: 'Connected', agents: $agents.filter((agent) => agent.presence === 'connected') },
    { name: 'Installed', agents: $agents.filter((agent) => agent.presence === 'found') },
  ].filter((group) => group.agents.length > 0);
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
    if (applying) return;
    rowErrors = { ...rowErrors, [agent.id]: '' };
    rowNotes = { ...rowNotes, [agent.id]: '' };
    try {
      const changes = await planConnect(agent.id, false);
      // An empty plan has nothing to show a review for; the row's own state
      // already says whether it is connected.
      if (changes.length === 0) return;
      reviewing = { agent, plan: changes };
      land(() => applyButton);
    } catch (error) {
      rowErrors = { ...rowErrors, [agent.id]: whyItFailed(error) };
    }
  }

  // The rows say what is connected, so the review closes on the fresh list
  // rather than on the one the apply has already made wrong.
  async function apply() {
    if (reviewing === null || applying) return;
    const id = reviewing.agent.id;
    const reviewing_name = reviewing.agent.name;
    applying = true;
    try {
      const note = await applyConnect(id, false);
      if (note) rowNotes = { ...rowNotes, [id]: note };
      const read = await refresh();
      reviewing = null;
      // The agent is connected, so its Connect button is gone. Its name, now in
      // the Connected group, is what says the result.
      land(() => nameOf(id));
      if (read) {
        announce(`${reviewing_name} is connected.`);
      } else {
        listNote = {
          text: `${reviewing_name} is connected. The list could not be read again, so what it shows may be out of date.`,
          failed: false,
        };
      }
    } catch (error) {
      rowErrors = { ...rowErrors, [id]: whyItFailed(error) };
      reviewing = null;
      land(() => connectOf(id) ?? nameOf(id));
    } finally {
      applying = false;
    }
  }

  function cancel() {
    if (applying) return;
    const id = reviewing?.agent.id;
    reviewing = null;
    if (id !== undefined) land(() => connectOf(id));
  }
</script>

{#if listNote}
  <div class="caveat" class:failed={listNote.failed} role="status">
    <p class="note">{listNote.text}</p>
    <button class="btn" on:click={look}>Look again</button>
  </div>
{/if}

{#each groups as group (group.name)}
  <Row name={group.name} leaves>
    <ul class="agents" aria-label={group.name}>
      {#each group.agents as agent (agent.id)}
        <!-- A review plans a connect, so it stays with the agent still to connect
             and never opens under the one it just moved. -->
        {@const open =
          reviewing?.agent.id === agent.id && agent.presence === 'found' ? reviewing : null}
        <li class="agent" data-travel={agent.id} out:leave={true}>
          <span class="name" id={`agent-${agent.id}`} tabindex="-1">{agent.name}</span>
          {#if agent.presence === 'found' && !open}
            <button
              class="btn"
              id={`connect-${agent.id}`}
              aria-disabled={applying ? 'true' : undefined}
              on:click={() => review(agent)}
            >
              Connect
            </button>
          {/if}
          {#if rowErrors[agent.id] || rowNotes[agent.id] || open}
            <div class="under">
              {#if rowErrors[agent.id]}<p class="error">{rowErrors[agent.id]}</p>{/if}
              {#if rowNotes[agent.id]}<p class="note pending">{rowNotes[agent.id]}</p>{/if}
              {#if open}
                <DiffReview plan={open.plan} {apply} {cancel} busy={applying} bind:applyButton />
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  </Row>
{:else}
  {#if !looked}
    <p class="lede">Looking for agents on this machine.</p>
  {:else if listNote === null}
    <p class="lede">No coding agent is installed on this machine.</p>
  {/if}
{/each}

{#if alsoWorksWith}
  <p class="note elsewhere" class:inset={groups.length > 0} data-travel="">
    Banshee also works with {alsoWorksWith}.
  </p>
{/if}

<style>
  .lede {
    max-width: 520px;
    margin: 0 0 20px;
    --wght: var(--cut-agent-weight);
    --wdth: var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
  }

  /* A caveat and a fault read the same shape; only the colour separates them,
     and the words carry the difference for anyone who cannot see it. */
  .caveat {
    margin: 0 0 16px;
  }

  .caveat p {
    margin: 0 0 10px;
    color: var(--dim);
  }

  .caveat.failed p {
    color: var(--accent);
  }

  .agents {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    min-width: 0;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Connect holds the right edge, where Play and Get stand on Speaking. */
  .agent {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    column-gap: 12px;
    min-height: var(--control-height);
    min-width: 0;
  }

  .name {
    --wght: var(--cut-head-weight);
    --wdth: var(--cut-head-width);
    font-size: 15px;
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .agent .btn {
    flex: none;
  }

  .under {
    flex: 0 0 100%;
    min-width: 0;
  }

  .elsewhere {
    margin: 0;
    color: var(--dim);
  }

  .inset {
    margin-left: calc(var(--label-column) + var(--label-gap));
  }

  .error {
    margin: 8px 0 0;
    font-size: 13px;
    color: var(--accent);
    /* The daemon's message carries the commands it did not run, one per line.
       Collapsed, they read as one run-on sentence. */
    white-space: pre-wrap;
    overflow-x: auto;
  }

  .under .note {
    margin: 8px 0 0;
  }
</style>
