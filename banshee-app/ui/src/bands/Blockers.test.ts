import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../lib/tauri', async () => (await import('../lib/tauri.mock')).mockTauri());

import Blockers from './Blockers.svelte';
import { claimTheRun, forgetTheAsk } from '../lib/downloads';
import { setSetting } from '../lib/tauri';
import permissions from '../mocks/permissions.json';
import type { Blocker, BlockerKind, ListeningModel } from '../lib/daemon';

beforeEach(forgetTheAsk);

const at = (bytes: number) => ({
  label: 'Listening model',
  model: 'ggml-x.bin',
  index: 1,
  count: 4,
  bytes,
  total: 100,
  state: 'downloading' as const,
});

// The daemon reports each percent. A reader watching the screen wants all of
// them; a reader listening wants four.
it('shows every percent and says only the quarters', async () => {
  const { container, rerender } = render(Blockers, {
    blockers: [],
    download: at(26),
    restart: () => {},
  });
  const shown = () => container.querySelector('.progress')?.textContent?.trim();
  const said = () => container.querySelector('[aria-live="polite"]')?.textContent?.trim();

  expect(shown()).toContain('26%');
  const first = said();

  await rerender({ blockers: [], download: at(49), restart: () => {} });
  expect(shown()).toContain('49%');
  expect(said()).toBe(first);

  await rerender({ blockers: [], download: at(51), restart: () => {} });
  expect(said()).not.toBe(first);
});

it('names the pane the System Settings button opens', () => {
  const { getByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
  });
  expect(getByRole('button', { name: /Open System Settings for Accessibility/ })).toBeTruthy();
});

// The title of a blocker is the most important line on the surface when one is
// there, and a span cannot be reached by heading.
it('titles a blocker with a heading', () => {
  const { getAllByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
  });
  // One thing to do carries no count: "1 of 1" sends the reader looking for a
  // second box that is not there.
  expect(getAllByRole('heading').map((h) => h.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
    'Accessibility',
  ]);
});

// A TCC grant reaches only a process started after it lands, so opening the
// pane cannot finish the job. The box has to offer the restart that does.
it('offers the restart that makes a granted permission real', async () => {
  let restarted = 0;
  const { getByRole, queryByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => (restarted += 1),
  });
  expect(queryByRole('button', { name: /I granted it/ })).toBeNull();

  await fireEvent.click(getByRole('button', { name: /Open System Settings for Accessibility/ }));
  await fireEvent.click(getByRole('button', { name: /I granted it/ }));
  expect(restarted).toBe(1);
});

// Two boxes with no count leave the reader holding "which one did I do" across
// an app switch. A first run holds this pair: the models arrive while the grant
// is still outstanding.
it('numbers each thing it still needs', () => {
  const { getAllByRole } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    download: at(26),
    restart: () => {},
  });
  const heads = getAllByRole('heading').map((h) => h.textContent?.replace(/\s+/g, ' ').trim());
  expect(heads[0]).toMatch(/^1 of 2 · Getting Banshee's models/);
  expect(heads[1]).toMatch(/^2 of 2 · Accessibility/);
});

// A model arriving repairs a model, so that box may stand down for the run. It
// brings no microphone, so a capture fault has to keep its place beside it.
it('keeps a microphone fault beside a running download, and stands a model restart down', () => {
  const restart = (kind: BlockerKind, name: string): Blocker => ({
    kind,
    id: 'recording_pipeline',
    name,
    remedy: 'restart',
    consequence: 'nothing can be recorded',
    fix: 'connect a microphone, then restart',
  });

  const microphone = render(Blockers, {
    blockers: [restart('pipeline', 'The microphone is not working')],
    download: at(26),
    restart: () => {},
  });
  expect(microphone.getByRole('heading', { name: /microphone is not working/i })).toBeTruthy();

  const model = render(Blockers, {
    blockers: [restart('model', 'Banshee needs a restart')],
    download: at(26),
    restart: () => {},
  });
  expect(model.queryByRole('heading', { name: /needs a restart/i })).toBeNull();
});

// The microphone headline sends the reader to their hardware.
it('names the remote listener when the fault is the server, not the microphone', () => {
  const { getByRole, queryByRole } = render(Blockers, {
    blockers: [
      {
        kind: 'provider',
        id: 'recording_pipeline',
        name: 'The remote listener is not reachable',
        remedy: 'restart',
        consequence: 'dictation and ask_user do not work until the key or server is fixed',
        fix: 'set the key: banshee config set stt.remote.api_key, or fix [stt.remote] base_url, then restart: banshee start',
        command: 'banshee start',
      } satisfies Blocker,
    ],
    restart: () => {},
  });
  expect(getByRole('heading', { name: /remote listener is not reachable/i })).toBeTruthy();
  expect(queryByRole('heading', { name: /microphone/i })).toBeNull();
  expect(getByRole('button', { name: 'Restart' })).toBeTruthy();
});

// Writing the key reads the same file first, so the headline sends the reader
// to the file.
it('names the key file when the fault is the file, not the server', () => {
  const { getByRole, queryByRole } = render(Blockers, {
    blockers: [
      {
        kind: 'keyfile',
        id: 'recording_pipeline',
        name: "The remote listener's key file is unreadable",
        remedy: 'restart',
        consequence: 'dictation and ask_user do not work until the key file is fixed',
        fix: 'rm /Users/someone/.banshee/credentials.toml, then set the keys again',
      } satisfies Blocker,
    ],
    restart: () => {},
  });
  expect(getByRole('heading', { name: /key file is unreadable/i })).toBeTruthy();
  expect(queryByRole('heading', { name: /not reachable/i })).toBeNull();
  expect(getByRole('button', { name: 'Restart anyway' })).toBeTruthy();
});

// First run is the one moment a person asks what Banshee is, and it is
// derived rather than stored: nothing dictated yet.
it('says what Banshee is on a first run, and not afterwards', () => {
  const { queryByText, rerender } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
    first: true,
  });
  expect(queryByText(/types what you say/)).toBeTruthy();

  rerender({ blockers: permissions.blockers as Blocker[], restart: () => {}, first: false });
  expect(queryByText(/types what you say/)).toBeNull();
});

// The daemon streams the percent, so the bar draws a real value rather than
// standing in for one.
it('draws the download against its own length', () => {
  const { container } = render(Blockers, {
    blockers: [],
    download: {
      label: 'Listening model',
      model: 'ggml.bin',
      index: 1,
      count: 4,
      bytes: 41,
      total: 100,
      state: 'downloading' as const,
    },
    restart: () => {},
  });
  expect(container.querySelector('.bar')?.getAttribute('style')).toMatch(/scaleX\(0\.41\)/);
});

// A failed file does not end the run: the daemon carries on to the next one and
// refuses a second run while the first holds the slot. Offering a retry here
// would be a button the daemon answers -32005.
it('offers no retry while the run that failed a file is still going', () => {
  const { queryByRole, container } = render(Blockers, {
    blockers: [],
    download: {
      label: 'Listening model',
      model: 'ggml.bin',
      index: 1,
      count: 4,
      bytes: 41,
      total: 100,
      state: 'failed' as const,
    },
    restart: () => {},
  });
  expect(queryByRole('button', { name: 'Try again' })).toBeNull();
  // The visible line names which file failed; the run itself carries on.
  expect(container.querySelector('.progress')?.textContent).toMatch(/failed/);
});

// `forget()` empties the record when saving is switched off, so an established
// install cannot be told apart from a new one by row count alone. Claiming a
// first run at someone months in is worse than staying quiet.
it('does not claim a first run when the record is off rather than empty', () => {
  const { queryByText } = render(Blockers, {
    blockers: permissions.blockers as Blocker[],
    restart: () => {},
    first: false,
  });
  expect(queryByText(/types what you say/)).toBeNull();
});

// A first run is the one moment a person asks what this is, so the line names
// every machine their words touch, and no machine it does not reach.
const OPENING: [string | null, string | null, string][] = [
  [
    'api.groq.com',
    'api.openai.com',
    'what you say goes to api.groq.com to be heard, and what it says comes from api.openai.com.',
  ],
  ['api.groq.com', null, 'what you say goes to api.groq.com to be heard.'],
  [null, 'api.openai.com', 'what it says comes from api.openai.com.'],
  [null, null, 'nothing you say leaves this machine.'],
];

it.each(OPENING)(
  'names the sides a first run reaches: %s and %s',
  (remoteHost, speechHost, says) => {
    const { getByText } = render(Blockers, {
      blockers: [
        {
          kind: 'permission',
          id: 'accessibility',
          name: 'Accessibility',
          consequence: 'dictation cannot type',
          fix: 'grant it in System Settings',
        },
      ],
      restart: () => {},
      first: true,
      remoteHost,
      speechHost,
    });
    // The paragraph wraps in the markup, so the line is read as one space run.
    const opening = getByText(/types what you say/).textContent?.replace(/\s+/g, ' ');
    expect(opening).toBe(
      `Banshee types what you say into whatever app you are using, and ${says} It needs one thing first.`,
    );
  },
);

const missingModel: Blocker = {
  kind: 'model' as BlockerKind,
  id: 'ggml-x.bin',
  name: 'ggml-x.bin',
  role: 'speech',
  remedy: 'download',
  consequence: 'recording does not work',
  fix: 'run: banshee setup',
  command: 'banshee setup',
};

it('shuts Download while a run is already asked for', () => {
  claimTheRun();
  const { getByRole } = render(Blockers, {
    blockers: [missingModel],
    download: null,
    restart: () => {},
  });

  expect((getByRole('button', { name: /Download/ }) as HTMLButtonElement).disabled).toBe(true);
});

it('offers Download while nothing is running', () => {
  const { getByRole } = render(Blockers, {
    blockers: [missingModel],
    download: null,
    restart: () => {},
  });

  expect((getByRole('button', { name: /Download/ }) as HTMLButtonElement).disabled).toBe(false);
});

const suggestion = (id: string, label: string, languages: string[]): ListeningModel => ({
  id,
  label,
  megabytes: 700,
  downloaded: false,
  suggested: true,
  languages,
  streams: false,
  credit: null,
  licence: { name: 'MIT', summary: 'Use it.' },
  accept_command: null,
});

// The model decides what the run costs, so the box asks before a download that
// runs to gigabytes, and says what each suggestion hears.
it('asks which listening model to fetch, from the suggestions alone', async () => {
  const { getByRole, getByText, queryByRole } = render(Blockers, {
    blockers: [missingModel],
    download: null,
    restart: () => {},
    megabytes: 1440,
    chosen: 'parakeet-unified-en-0.6b',
    models: [
      suggestion('parakeet-unified-en-0.6b', 'Parakeet Unified', ['en']),
      suggestion('whisper-large-v3-turbo', 'Whisper turbo', ['af', 'am', 'ar', 'as']),
      { ...suggestion('gigaam-v3-ctc', 'gigaam-v3-ctc', ['ru']), suggested: false },
    ],
  });

  const group = getByRole('radiogroup', { name: 'Listening model' });
  expect(
    [...group.querySelectorAll('[role="radio"]')].map((one) => one.textContent?.trim()),
  ).toEqual(['Parakeet Unified', 'Whisper turbo']);
  expect(getByRole('radio', { name: 'Parakeet Unified' }).getAttribute('aria-checked')).toBe(
    'true',
  );
  expect(
    getByText(
      'About 1.4 GB to fetch. Parakeet Unified hears English. Whisper turbo hears 4 languages. You can change this later.',
    ),
  ).toBeTruthy();
  expect(queryByRole('radiogroup', { name: 'Speech model' })).toBeNull();

  await fireEvent.click(getByRole('radio', { name: 'Whisper turbo' }));
  expect(vi.mocked(setSetting)).toHaveBeenCalledWith('stt.model', 'whisper-large-v3-turbo');
});

// The window offers no acceptance, so a model set through the CLI shows its
// licence here too, and a Download that would end in "failed" stays hidden.
it('shows the licence a chosen model waits on, and offers no Download', () => {
  const canary: ListeningModel = {
    ...suggestion('canary-1b', 'canary-1b', ['en', 'de', 'es', 'fr']),
    suggested: false,
    licence: { name: 'CC-BY-NC-4.0', summary: 'Non-commercial use only.' },
    accept_command: 'banshee setup --model canary-1b --accept-licence',
  };
  const { getByText, queryByRole } = render(Blockers, {
    blockers: [missingModel],
    download: null,
    restart: () => {},
    chosen: 'canary-1b',
    models: [suggestion('parakeet-unified-en-0.6b', 'Parakeet Unified', ['en']), canary],
  });

  expect(
    getByText('Banshee will not download canary-1b until you accept its licence, CC-BY-NC-4.0.'),
  ).toBeTruthy();
  expect(getByText('banshee setup --model canary-1b --accept-licence')).toBeTruthy();
  expect(queryByRole('button', { name: 'Download' })).toBeNull();
});
