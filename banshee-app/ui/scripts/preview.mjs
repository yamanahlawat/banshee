// Captures the window in a real browser against the mock bridge, so visual
// work gets eyes without the daemon. Needs the dev server first:
//
//   npm run dev -- --port 5173 --strictPort
//   node scripts/preview.mjs [out-dir]
//
// Uses the system Chrome. Point BANSHEE_CHROME elsewhere when it lives in
// another place. `puppeteer-core` drives it, so no browser downloads.
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(path.join(here, '..', 'package.json'));
const puppeteer = require('puppeteer-core');

const DIR = process.argv[2] ?? '/tmp/shots';
fs.mkdirSync(DIR, { recursive: true });

const executablePath =
  process.env.BANSHEE_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
if (!fs.existsSync(executablePath)) {
  console.error(`No Chrome at ${executablePath}. Set BANSHEE_CHROME to its path.`);
  process.exit(1);
}

const browser = await puppeteer.launch({
  executablePath,
  headless: 'new',
  args: ['--no-sandbox', '--disable-dev-shm-usage', '--force-color-profile=srgb'],
});

const states = ['ready', 'first-run', 'downloading', 'armed', 'stt-failed'];
for (const scheme of ['light', 'dark']) {
  for (const state of states) {
    // Downloading only animates in light once; dark covers steady states.
    if (scheme === 'dark' && (state === 'downloading' || state === 'armed')) continue;
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(`console: ${m.text()}`);
    });
    await page.setViewport({ width: 480, height: 900, deviceScaleFactor: 2 });
    await page.emulateMediaFeatures([
      { name: 'prefers-color-scheme', value: scheme },
      { name: 'prefers-reduced-motion', value: 'reduce' },
    ]);
    await page.goto(`http://localhost:5173/?state=${state}`, { waitUntil: 'networkidle0' });
    await new Promise((r) => setTimeout(r, state === 'downloading' ? 2500 : 1200));
    const file = path.join(DIR, `${state}-${scheme}.png`);
    await page.screenshot({ path: file });
    console.log(file, errors.length ? JSON.stringify(errors) : 'clean');
    await page.close();
  }
}
await browser.close();
