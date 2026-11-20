import { fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { beforeEach, expect, it, onTestFinished, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());
vi.mock(import('../lib/travel'), async (actual) => ({
  ...(await actual()),
  travel: vi.fn(),
}));

import { applyConnect, detectAgents, planConnect, type AgentRow } from '../lib/tauri';
import { agents } from '../lib/agents';
import { announcement } from '../lib/copy';
import { travel } from '../lib/travel';
import AgentsPage from './AgentsPage.svelte';

const CLAUDE: AgentRow = {
  id: 'claude',
  name: 'Claude Code',
  presence: 'found',
  note: 'Installed, not connected',
};

beforeEach(() => {
  agents.set([]);
  vi.mocked(detectAgents).mockReset();
  vi.mocked(planConnect).mockReset();
  vi.mocked(applyConnect).mockReset();
  vi.mocked(travel).mockClear();
});

// A read that failed is not a machine with no agents. Left saying it is still
// looking, the page reports a green check the daemon never gave, and this
// audience has no terminal to fall back to.
it('says the list could not be read, and offers to read it again', async () => {
  vi.mocked(detectAgents).mockRejectedValueOnce(new Error('no daemon'));
  const { getByRole, getByText, queryByText } = render(AgentsPage);

  await waitFor(() =>
    expect(
      getByText(
        'Banshee could not read which agents are installed. Banshee is probably not running.',
      ),
    ).toBeTruthy(),
  );
  expect(queryByText(/daemon/i)).toBeNull();
  expect(queryByText(/Looking for agents/)).toBeNull();

  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  await fireEvent.click(getByRole('button', { name: /Look again/ }));

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  expect(queryByText(/could not read which agents/i)).toBeNull();
});

// An empty list after a read that landed is a real answer, and a different one.
it('says the machine has no agents once a read has landed', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([]);
  const { getByText, queryByText } = render(AgentsPage);

  await waitFor(() => expect(getByText(/No coding agent is installed/i)).toBeTruthy());
  expect(queryByText(/Looking for agents/)).toBeNull();
});

// The write landed, so the page may not report a failure. Only the list it
// draws is doubtful, and it says which of the two happened.
it('states that the agent connected when the list cannot be read afterwards', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockResolvedValue(null);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Apply' })).toBeTruthy());

  vi.mocked(detectAgents).mockRejectedValueOnce(new Error('no daemon'));
  await fireEvent.click(getByRole('button', { name: 'Apply' }));

  await waitFor(() => expect(getByText(/Claude Code is connected/)).toBeTruthy());
  expect(getByText(/may be out of date/)).toBeTruthy();
});

// The connect landed, so a failed re-read adds no second line for a screen reader.
it('announces nothing beyond the caveat when the list cannot be read after an apply', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockResolvedValue(null);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Apply' })).toBeTruthy());

  announcement.set('');
  const said: string[] = [];
  const stop = announcement.subscribe((line) => line && said.push(line));
  vi.mocked(detectAgents).mockRejectedValueOnce(new Error('no daemon'));
  await fireEvent.click(getByRole('button', { name: 'Apply' }));

  await waitFor(() => expect(getByText(/may be out of date/)).toBeTruthy());
  stop();
  expect(said).toEqual([]);
});

// Each of the three below destroys the control that was just pressed. axe
// cannot see a lost focus, so it is asserted here.
const CURSOR: AgentRow = { id: 'cursor', name: 'Cursor', presence: 'found', note: '' };
const CONNECTED: AgentRow = { ...CLAUDE, presence: 'connected', note: '' };

it('moves focus into the review when the plan opens', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));

  await waitFor(() => expect(document.activeElement).toBe(getByRole('button', { name: 'Apply' })));
});

it('puts focus back on the connect button the cancel came from', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Cancel' })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Cancel' }));

  await waitFor(() =>
    expect(document.activeElement).toBe(getByRole('button', { name: 'Connect' })),
  );
});

// The agent is connected now, so its Connect button is gone, and its name has
// moved to the Connected group.
it('moves focus onto the name an apply just moved, and says so once', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockResolvedValue(null);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Apply' })).toBeTruthy());

  announcement.set('');
  const said: string[] = [];
  const stop = announcement.subscribe((line) => line && said.push(line));
  vi.mocked(detectAgents).mockResolvedValueOnce([CONNECTED]);
  await fireEvent.click(getByRole('button', { name: 'Apply' }));

  const connected = await waitFor(() => getByRole('list', { name: 'Connected' }));
  const name = within(connected).getByText('Claude Code');
  await waitFor(() => expect(document.activeElement).toBe(name));
  expect(name.tabIndex).toBe(-1);
  stop();
  expect(said).toEqual(['Claude Code is connected.']);
});

it('moves the agent an apply connected to its new group, and nothing on a read alone', async () => {
  vi.mocked(detectAgents).mockResolvedValue([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockResolvedValue(null);
  const { getByRole, getByText } = render(AgentsPage);
  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await waitFor(() => expect(getByRole('button', { name: 'Apply' })).toBeTruthy());
  expect(travel).not.toHaveBeenCalled();

  vi.mocked(detectAgents).mockResolvedValue([CONNECTED]);
  await fireEvent.click(getByRole('button', { name: 'Apply' }));
  await waitFor(() => getByRole('list', { name: 'Connected' }));

  expect(vi.mocked(travel).mock.calls).toEqual([['claude']]);
});

// A failed apply leaves the agent unconnected, so Connect is where the reader
// was; when a re-read has taken Connect away, the agent's name is.
it("puts focus back on that agent's Connect after a failed apply", async () => {
  vi.mocked(detectAgents).mockResolvedValue([CLAUDE, CURSOR]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.cursor/mcp.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockRejectedValue(new Error('config changed after the plan'));
  const { getAllByRole, getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getAllByRole('button', { name: 'Connect' })).toHaveLength(2));
  await fireEvent.click(getAllByRole('button', { name: 'Connect' })[1]);
  await fireEvent.click(await waitFor(() => getByRole('button', { name: 'Apply' })));

  await waitFor(() => expect(getByText(/changed after the plan/)).toBeTruthy());
  const connect = getAllByRole('button', { name: 'Connect' })[1];
  await waitFor(() => expect(document.activeElement).toBe(connect));
  expect(connect.id).toBe('connect-cursor');
});

it("puts focus on the agent's name after a failed apply when Connect is gone", async () => {
  vi.mocked(detectAgents).mockResolvedValue([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockImplementation(async () => {
    agents.set([CONNECTED]);
    throw new Error('config changed after the plan');
  });
  const { getByRole, getByText, queryByRole } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await fireEvent.click(await waitFor(() => getByRole('button', { name: 'Apply' })));

  await waitFor(() => expect(getByText(/changed after the plan/)).toBeTruthy());
  expect(queryByRole('button', { name: 'Connect' })).toBeNull();
  await waitFor(() => expect(document.activeElement).toBe(getByText('Claude Code')));
});

const CODEX: AgentRow = {
  id: 'codex',
  name: 'Codex',
  presence: 'found',
  note: 'Installed, not connected',
};
const TRUST = 'Codex runs this hook only after you trust it: open Codex and run /hooks.';

// Codex skips a hook nobody trusted. A window user has no terminal line that says so.
it('shows what is left to do after a connect that needs a step', async () => {
  vi.mocked(detectAgents)
    .mockResolvedValueOnce([CODEX])
    .mockResolvedValueOnce([{ ...CODEX, presence: 'connected', note: 'Connected' }]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.codex/hooks.json', diff: '+ turn-end' }]);
  vi.mocked(applyConnect).mockResolvedValue(TRUST);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));

  await waitFor(() => expect(getByText(TRUST)).toBeTruthy());
});

it('shows no step after a connect that failed', async () => {
  vi.mocked(detectAgents).mockResolvedValue([CODEX]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.codex/hooks.json', diff: '+ turn-end' }]);
  vi.mocked(applyConnect).mockRejectedValue(new Error('config.toml changed after the plan'));
  const { getByRole, getByText, queryByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));

  await waitFor(() => expect(getByText(/changed after the plan/)).toBeTruthy());
  expect(queryByText(TRUST)).toBeNull();
});

it('drops the step when a later connect fails', async () => {
  vi.mocked(detectAgents)
    .mockResolvedValueOnce([CODEX])
    .mockRejectedValueOnce(new Error('no daemon'));
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.codex/hooks.json', diff: '+ turn-end' }]);
  vi.mocked(applyConnect)
    .mockResolvedValueOnce(TRUST)
    .mockRejectedValueOnce(new Error('config.toml changed after the plan'));
  const { getByRole, getByText, queryByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));
  await waitFor(() => expect(getByText(TRUST)).toBeTruthy());

  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Apply/ }));

  await waitFor(() => expect(getByText(/changed after the plan/)).toBeTruthy());
  expect(queryByText(TRUST)).toBeNull();
});

// A long diff scrolls inside its own box, and a keyboard user has no wheel.
it('lets the keyboard reach each change in the review by its name', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CODEX]);
  vi.mocked(planConnect).mockResolvedValue([
    { path: '~/.codex/hooks.json', diff: '+ turn-end' },
    { path: null, diff: '$ codex mcp add banshee' },
  ]);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());

  const file = getByRole('region', { name: 'Changes to ~/.codex/hooks.json' });
  const command = getByRole('region', { name: 'Command to run' });
  expect(file.textContent).toBe('+ turn-end');
  expect(file.tabIndex).toBe(0);
  expect(command.tabIndex).toBe(0);
});

// The header repeats the path shown above the box, and `--- /dev/null` is the
// only other sign of a new file.
it("marks a new file beside its path and drops both diff's headers", async () => {
  const created = {
    path: '~/.codex/hooks.json',
    diff: '--- /dev/null\n+++ b/.codex/hooks.json\n@@ -0,0 +1,2 @@\n+line one\n+line two\n',
  };
  const changed = {
    path: '~/.codex/config.toml',
    diff: '--- a/.codex/config.toml\n+++ b/.codex/config.toml\n@@ -1,2 +1,2 @@\n-old\n+new\n',
  };
  vi.mocked(detectAgents).mockResolvedValueOnce([CODEX]);
  vi.mocked(planConnect).mockResolvedValue([created, changed]);
  const { getByRole, getByText, queryByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await waitFor(() => expect(getByRole('button', { name: /Apply/ })).toBeTruthy());

  expect(getByText('~/.codex/hooks.json (new file)')).toBeTruthy();
  expect(getByText('~/.codex/config.toml')).toBeTruthy();
  expect(queryByText('~/.codex/config.toml (new file)')).toBeNull();

  const file = getByRole('region', { name: 'New file ~/.codex/hooks.json' });
  const other = getByRole('region', { name: 'Changes to ~/.codex/config.toml' });
  expect(file.textContent).not.toContain('/dev/null');
  expect(file.textContent).not.toContain('+++');
  expect(other.textContent).not.toContain('---');
  expect(other.textContent).not.toContain('+++');
});

const OPENCODE: AgentRow = { id: 'opencode', name: 'OpenCode', presence: 'connected', note: '' };
const PI: AgentRow = { id: 'pi', name: 'Pi', presence: 'absent', note: '' };

function names(list: HTMLElement): string[] {
  return within(list)
    .getAllByRole('listitem')
    .map((item) => item.querySelector('.name')?.textContent?.trim() ?? '');
}

// The daemon lists agents alphabetically, and each group keeps that order.
it('groups the agents by state, Connected first, each in the order the daemon lists them', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([
    CONNECTED,
    { ...CODEX, presence: 'connected', note: '' },
    CURSOR,
    OPENCODE,
    PI,
    { ...CODEX, id: 'zed', name: 'Zed' },
  ]);
  const { getByRole } = render(AgentsPage);

  const connected = await waitFor(() => getByRole('list', { name: 'Connected' }));
  const installed = getByRole('list', { name: 'Installed' });
  expect(names(connected)).toEqual(['Claude Code', 'Codex', 'OpenCode']);
  expect(names(installed)).toEqual(['Cursor', 'Zed']);
  expect(within(connected).queryByRole('button', { name: 'Connect' })).toBeNull();
  expect(within(installed).getAllByRole('button', { name: 'Connect' })).toHaveLength(2);
  expect(
    connected.compareDocumentPosition(installed) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
});

it('draws no group that holds no agent', async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CONNECTED, PI]);
  const { getByRole, queryByRole, queryByText } = render(AgentsPage);

  await waitFor(() => expect(getByRole('list', { name: 'Connected' })).toBeTruthy());
  expect(queryByRole('list', { name: 'Installed' })).toBeNull();
  expect(queryByText('Installed')).toBeNull();
});

// Each says something about one agent, so it stands under that agent's name.
it("puts the review under the agent's own name", async () => {
  vi.mocked(detectAgents).mockResolvedValueOnce([CLAUDE, CURSOR]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.cursor/mcp.json', diff: '+ banshee' }]);
  const { getAllByRole, getByRole } = render(AgentsPage);

  await waitFor(() => expect(getAllByRole('button', { name: 'Connect' })).toHaveLength(2));
  await fireEvent.click(getAllByRole('button', { name: 'Connect' })[1]);
  const apply = await waitFor(() => getByRole('button', { name: 'Apply' }));

  const item = apply.closest('li');
  expect(item?.querySelector('.name')?.textContent).toBe('Cursor');
  expect(within(item as HTMLElement).getByText('What this changes')).toBeTruthy();
  expect(getByRole('list', { name: 'Installed' }).contains(apply)).toBe(true);
});

it("puts a refused connect under the agent's own name", async () => {
  vi.mocked(detectAgents).mockResolvedValue([CLAUDE, CURSOR]);
  vi.mocked(planConnect).mockRejectedValue(new Error('the config is not writable'));
  const { getAllByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getAllByRole('button', { name: 'Connect' })).toHaveLength(2));
  await fireEvent.click(getAllByRole('button', { name: 'Connect' })[0]);

  const error = await waitFor(() => getByText('The config is not writable.'));
  expect(error.closest('li')?.querySelector('.name')?.textContent).toBe('Claude Code');
});

// An agent that comes back while its row fades out keeps that row, which has
// to stand in the flow again.
it('puts a row back in the list when its agent returns mid-fade', async () => {
  const held = vi.spyOn(Element.prototype, 'animate').mockImplementation(
    () =>
      ({
        cancel() {},
        effect: null,
        currentTime: 0,
        playState: 'running',
        finished: new Promise(() => {}),
        onfinish: null,
      }) as unknown as Animation,
  );
  onTestFinished(() => held.mockRestore());
  vi.mocked(detectAgents).mockResolvedValue([CLAUDE, CURSOR]);
  const { getByText } = render(AgentsPage);
  await waitFor(() => expect(getByText('Cursor')).toBeTruthy());
  const row = getByText('Cursor').closest('li') as HTMLElement;

  agents.set([CLAUDE]);
  await waitFor(() => expect(row.style.position).toBe('absolute'));
  agents.set([CLAUDE, CURSOR]);

  await waitFor(() => expect(row.style.position).toBe(''));
  expect(getByText('Cursor').closest('li')).toBe(row);
  expect(row.style.width).toBe('');
});

// The socket's own words name a daemon, which the window never says.
it('says Banshee is not running when a review cannot reach it', async () => {
  vi.mocked(detectAgents).mockResolvedValue([CLAUDE]);
  vi.mocked(planConnect).mockRejectedValue({
    message: 'the daemon closed the socket',
    transport: true,
    sent: false,
  });
  const { getByRole, getByText, queryByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));

  await waitFor(() => expect(getByText('Banshee is not running.')).toBeTruthy());
  expect(queryByText(/socket/)).toBeNull();
});

it('says Banshee did not answer when an apply left and no reply came', async () => {
  vi.mocked(detectAgents).mockResolvedValue([CLAUDE]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  vi.mocked(applyConnect).mockRejectedValue({
    message: 'the daemon closed the socket',
    transport: true,
    sent: true,
  });
  const { getByRole, getByText, queryByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Claude Code')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: 'Connect' }));
  await fireEvent.click(await waitFor(() => getByRole('button', { name: 'Apply' })));

  await waitFor(() => expect(getByText('Banshee did not answer.')).toBeTruthy());
  expect(queryByText(/socket/)).toBeNull();
});

it("puts what is left to do under the connected agent's name", async () => {
  vi.mocked(detectAgents)
    .mockResolvedValueOnce([CODEX])
    .mockResolvedValueOnce([{ ...CODEX, presence: 'connected', note: 'Connected' }]);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.codex/hooks.json', diff: '+ turn-end' }]);
  vi.mocked(applyConnect).mockResolvedValue(TRUST);
  const { getByRole, getByText } = render(AgentsPage);

  await waitFor(() => expect(getByText('Codex')).toBeTruthy());
  await fireEvent.click(getByRole('button', { name: /Connect/ }));
  await fireEvent.click(await waitFor(() => getByRole('button', { name: /Apply/ })));

  const step = await waitFor(() => getByText(TRUST));
  expect(getByRole('list', { name: 'Connected' }).contains(step)).toBe(true);
  expect(step.closest('li')?.querySelector('.name')?.textContent).toBe('Codex');
});

/// Opens the review of the first agent listed.
async function openTheReview(listed: AgentRow[] = [CLAUDE]) {
  vi.mocked(detectAgents).mockResolvedValueOnce(listed);
  vi.mocked(planConnect).mockResolvedValue([{ path: '~/.claude.json', diff: '+ banshee' }]);
  const page = render(AgentsPage);
  await waitFor(() => expect(page.getByText(listed[0].name)).toBeTruthy());
  await fireEvent.click(page.getAllByRole('button', { name: 'Connect' })[0]);
  await waitFor(() => expect(page.getByRole('button', { name: 'Apply' })).toBeTruthy());
  return page;
}

/// Answers the call that lets the apply land, with the list read again after it.
function anApplyTheDaemonHolds(): () => void {
  let answer: (note: null) => void = () => {};
  vi.mocked(applyConnect).mockReturnValue(new Promise((done) => (answer = done)));
  return () => {
    vi.mocked(detectAgents).mockResolvedValueOnce([CONNECTED]);
    answer(null);
  };
}

it('says an apply is running until it lands, and keeps focus on the button', async () => {
  const land = anApplyTheDaemonHolds();
  const { getByRole, queryByRole } = await openTheReview();
  const apply = getByRole('button', { name: 'Apply' });
  apply.focus();

  await fireEvent.click(apply);

  const applying = getByRole('button', { name: 'Applying' });
  expect(applying.getAttribute('aria-disabled')).toBe('true');
  expect(document.activeElement).toBe(applying);
  land();
  await waitFor(() => expect(queryByRole('button', { name: 'Applying' })).toBeNull());
});

it('connects once when Apply is pressed again while the apply runs', async () => {
  const land = anApplyTheDaemonHolds();
  const { getByRole } = await openTheReview();

  await fireEvent.click(getByRole('button', { name: 'Apply' }));
  await fireEvent.click(getByRole('button', { name: 'Applying' }));

  expect(applyConnect).toHaveBeenCalledTimes(1);
  land();
  await waitFor(() => expect(getByRole('list', { name: 'Connected' })).toBeTruthy());
});

it('keeps the review open when Cancel is pressed while the apply runs', async () => {
  const land = anApplyTheDaemonHolds();
  const { getByRole } = await openTheReview();

  await fireEvent.click(getByRole('button', { name: 'Apply' }));
  const cancel = getByRole('button', { name: 'Cancel' });
  await fireEvent.click(cancel);

  expect(cancel.getAttribute('aria-disabled')).toBe('true');
  expect(getByRole('button', { name: 'Applying' })).toBeTruthy();
  land();
  await waitFor(() => expect(getByRole('list', { name: 'Connected' })).toBeTruthy());
});

it('opens no second review while an apply runs', async () => {
  const land = anApplyTheDaemonHolds();
  const { getByRole } = await openTheReview([CLAUDE, CURSOR]);

  await fireEvent.click(getByRole('button', { name: 'Apply' }));
  const other = getByRole('button', { name: 'Connect' });
  await fireEvent.click(other);

  expect(other.getAttribute('aria-disabled')).toBe('true');
  expect(planConnect).toHaveBeenCalledTimes(1);
  land();
  await waitFor(() => expect(getByRole('list', { name: 'Connected' })).toBeTruthy());
});

it('holds Apply at the wider of its two labels', async () => {
  const { getByRole } = await openTheReview();

  const held = getByRole('button', { name: 'Apply' }).querySelector('[aria-hidden="true"]');

  expect(held?.textContent).toBe('Applying');
});
