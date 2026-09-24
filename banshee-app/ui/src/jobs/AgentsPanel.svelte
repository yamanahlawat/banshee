<script lang="ts">
  import { onMount } from 'svelte';
  import { land } from '../lib/focus';
  import { applyConnect, planConnect, type AgentRow, type PlannedChange } from '../lib/tauri';
  import { agents, refresh } from '../lib/agents';

  let reviewing: { agent: AgentRow; plan: PlannedChange[] } | null = null;
  let rowErrors: Record<string, string> = {};
  /// What is left to do after a connect, per agent, as the daemon said it.
  let rowNotes: Record<string, string> = {};
  /// What the list has to say about itself. `failed` separates a fault from a
  /// caveat, because the accent is the colour of something being wrong.
  let listNote: { text: string; failed: boolean } | null = null;
  /// Whether a read has answered at all. An empty list before the first answer
  /// is a panel still looking; after one, it is a machine with no agents.
  let looked = false;

  let applyButton: HTMLButtonElement;
  const connectButtons: Record<string, HTMLButtonElement> = {};
  const rows: Record<string, HTMLElement> = {};

  onMount(look);

  /// Detection answering `false` is not an empty machine. A panel that keeps
  /// saying it is looking would report a green check the daemon never gave.
  async function look() {
    listNote = null;
    const read = await refresh();
    looked = true;
    if (!read) {
      listNote = {
        text: 'Banshee could not read which agents are installed. It asks the daemon for that, so the daemon is probably not running.',
        failed: true,
      };
    }
  }

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
    rowNotes = { ...rowNotes, [agent.id]: '' };
    try {
      const changes = await planConnect(agent.id, false);
      // An empty plan has nothing to show a review for; the row's own state
      // already says whether it is connected.
      if (changes.length === 0) return;
      reviewing = { agent, plan: changes };
      land(() => applyButton);
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
    const reviewing_name = reviewing.agent.name;
    try {
      const note = await applyConnect(id, false);
      if (note) rowNotes = { ...rowNotes, [id]: note };
      const read = await refresh();
      reviewing = null;
      // The agent is connected, so its Connect button is gone. The row is what
      // now says the result.
      land(() => rows[id]);
      if (!read) {
        listNote = {
          text: `${reviewing_name} is connected. The list could not be read again, so what it shows may be out of date.`,
          failed: false,
        };
      }
    } catch (error) {
      rowErrors = {
        ...rowErrors,
        [id]: (error as { message?: string })?.message || 'That failed.',
      };
      reviewing = null;
      land(() => connectButtons[id] ?? rows[id]);
    }
  }

  function cancel() {
    const id = reviewing?.agent.id;
    reviewing = null;
    if (id !== undefined) land(() => connectButtons[id]);
  }

  // The path above the box already names the file, so both header lines go.
  function body(diff: string) {
    const created = /^--- \/dev\/null\n/.test(diff);
    return {
      created,
      lines: diff
        .replace(/^--- .*\n\+\+\+ .*\n/, '')
        .replace(/\n$/, '')
        .split('\n'),
    };
  }
</script>

{#if reviewing}
  <div class="review">
    <span class="caps">What this changes</span>
    {#each reviewing.plan as change (change.diff)}
      {@const view = body(change.diff)}
      {#if change.path}<p class="path mono">
          {change.path}{view.created ? ' (new file)' : ''}
        </p>{/if}
      <!-- A box that scrolls needs a tab stop, or a keyboard cannot scroll it. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <pre
        class="diff"
        role="region"
        tabindex="0"
        aria-label={change.path
          ? view.created
            ? `New file ${change.path}`
            : `Changes to ${change.path}`
          : 'Command to run'}>{#each view.lines as line, i (i)}<span class="line">{line}</span
          >{/each}</pre>
    {/each}
    <div class="actions">
      <button class="btn" bind:this={applyButton} on:click={apply}>Apply</button>
      <button class="btn btn-ghost" on:click={cancel}>Cancel</button>
    </div>
  </div>
{:else}
  {#if listNote}
    <div class="caveat" class:failed={listNote.failed} role="status">
      <p>{listNote.text}</p>
      <button class="btn" on:click={look}>Look again</button>
    </div>
  {/if}
  <div class="rows">
    {#each here as agent (agent.id)}
      <div class="agent" tabindex="-1" bind:this={rows[agent.id]}>
        <div class="row">
          <span class="name">{agent.name}</span>
          <span class="presence caps" class:on={agent.presence === 'connected'}>
            {SAYS[agent.presence] ?? agent.presence}
          </span>
          {#if agent.presence === 'found'}
            <button class="btn" bind:this={connectButtons[agent.id]} on:click={() => review(agent)}>
              Connect
            </button>
          {/if}
        </div>
        {#if rowErrors[agent.id]}<p class="error">{rowErrors[agent.id]}</p>{/if}
        {#if rowNotes[agent.id]}<p class="note pending">{rowNotes[agent.id]}</p>{/if}
      </div>
    {:else}
      {#if !looked}
        <p class="lede">Looking for agents on this machine.</p>
      {:else if listNote === null}
        <p class="lede">No coding agent is installed on this machine.</p>
      {/if}
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
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 15px;
    line-height: 1.45;
  }

  /* A caveat and a fault read the same shape; only the colour separates them,
     and the words carry the difference for anyone who cannot see it. */
  .caveat {
    margin: 0 0 16px;
  }

  .caveat p {
    max-width: 520px;
    margin: 0 0 10px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--dim);
  }

  .caveat.failed p {
    color: var(--accent);
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
    font-variation-settings:
      'wght' 700,
      'wdth' 105;
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
    font-variation-settings:
      'wght' var(--cut-agent-weight),
      'wdth' var(--cut-agent-width);
    font-size: 13px;
    color: var(--dim);
  }

  .error {
    margin: 0 0 10px;
    font-size: 13px;
    color: var(--accent);
    /* The daemon's message carries the commands it did not run, one per line.
       Collapsed, they read as one run-on sentence. */
    white-space: pre-wrap;
    overflow-x: auto;
  }

  .note {
    margin: 0 0 10px;
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
    /* A whole number of rows minus half a row, so the last one is always cut
       and the box shows that it scrolls. Half the window, less a row, keeps
       Apply above the foot. */
    max-height: calc(round(down, min(36em, 50vh - 1.5em), 1.5em) - 0.75em);
    overflow: auto;
    border-left: 1px solid var(--rule);
    padding-left: 12px;
    font-variant-ligatures: none;
  }

  .line {
    display: block;
    min-height: 1.5em;
    padding-left: 1.2em;
    text-indent: -1.2em;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 18px;
  }

  .actions .btn {
    scroll-margin: 6px;
  }
</style>
