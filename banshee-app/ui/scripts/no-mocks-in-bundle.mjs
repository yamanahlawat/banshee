import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = fileURLToPath(new URL('..', import.meta.url));
const MOCKS = 'src/mocks';
const BUNDLE = 'dist';
const OPAQUE = ['.woff2', '.woff', '.ttf', '.png', '.jpg', '.jpeg', '.webp', '.ico', '.wasm'];

// A literal qualifies when the mocks hold it and the window's own source never
// would: the daemon names the device, the clipboard plugin words its own
// failure, and nobody types a dictation vocabulary into the window. Every mock
// file but `not-running.json` holds one, so a new file needs an entry only when
// none of these reaches it. `{"running": false}` can hold none, and the lint
// zone is what keeps that file out of the bundle.
const ONLY_IN_MOCKS = [
  'MacBook Pro Microphone',
  'OnePlus Buds 3',
  'the clipboard refused it',
  'pydantic ai',
];

function filesUnder(dir) {
  const found = [];
  for (const name of readdirSync(join(UI, dir))) {
    const path = join(dir, name);
    if (statSync(join(UI, path)).isDirectory()) found.push(...filesUnder(path));
    else found.push(path);
  }
  return found;
}

function textOf(paths) {
  return paths
    .filter((path) => !OPAQUE.some((extension) => path.endsWith(extension)))
    .map((path) => ({ path, body: readFileSync(join(UI, path), 'utf8') }));
}

function refuse(lines) {
  for (const line of lines) console.error(line);
  process.exit(1);
}

let bundle = [];
try {
  bundle = textOf(filesUnder(BUNDLE));
} catch {
  // An absent directory is the same absence as an empty one, refused below.
}
if (!bundle.some(({ path }) => path.endsWith('.js'))) {
  refuse([`No JavaScript under ${BUNDLE}/, so this check reads nothing. Build first.`]);
}

const mocks = textOf(filesUnder(MOCKS));
const unguarded = ONLY_IN_MOCKS.filter(
  (literal) => !mocks.some(({ body }) => body.includes(literal)),
);
if (unguarded.length > 0) {
  refuse([
    'This check guards nothing.',
    '',
    ...unguarded.map((literal) => `  "${literal}" is no longer in the mocks.`),
    '',
    `Pick a literal the mocks still hold and put it in ONLY_IN_MOCKS, in ${relative(
      UI,
      fileURLToPath(import.meta.url),
    )}.`,
  ]);
}

const shipped = bundle.flatMap(({ path, body }) =>
  ONLY_IN_MOCKS.filter((literal) => body.includes(literal)).map((literal) => ({ path, literal })),
);
if (shipped.length > 0) {
  refuse([
    'The mock harness is in the production bundle.',
    '',
    ...shipped.map(
      ({ path, literal }) => `  ${path} holds "${literal}", which only the mocks contain.`,
    ),
    '',
    'The mocks answer for an absent Tauri bridge in a dev build. src/lib/tauri.ts',
    'reaches them behind `import.meta.env.DEV`. Vite replaces that with `false` in a',
    'build, so the branch and its dynamic import both go. Two things break that:',
    'a changed guard in src/lib/tauri.ts, or another module under src/ that imports',
    'the mocks. Read both.',
  ]);
}

console.log(`No mock literal reached ${BUNDLE}/. Checked ${ONLY_IN_MOCKS.length} of them.`);
