import { expect, it } from 'vitest';
import tokens from './tokens.css?raw';

const components = import.meta.glob('../**/*.svelte', {
  query: '?raw',
  import: 'default',
  eager: true,
});

// Every rule in the window's styles, as [where, selector, body].
const rules = [
  ['tokens.css', tokens] as const,
  ...Object.entries(components).map(
    ([path, source]) =>
      [path, (source as string).match(/<style>([\s\S]*)<\/style>/)?.[1] ?? ''] as const,
  ),
].flatMap(([path, css]) =>
  [...css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(
    ([, selector, body]) => [path, selector.trim(), body] as const,
  ),
);

const ruleFor = (selector: string) =>
  rules
    .filter(([path, named]) => path === 'tokens.css' && named === selector)
    .map(([, , body]) => body)
    .join('');

// jsdom computes no inherited cut, so the rule is read from the stylesheet.
// The mono time takes the weight of whatever holds it, and every holder is 400.
it.each([
  ['.mono', '400'],
  ['.btn', '600'],
  ['.btn-underline', '600'],
])('sets %s in its own cut at %s, whatever the cut around it', (selector, weight) => {
  const body = ruleFor(selector);
  expect(body).toMatch(new RegExp(`\\n\\s*--wght: ${weight};`));
  expect(body).toMatch(/\n\s*--wdth: 100;/);
  if (/font-weight:/.test(body)) expect(body).toMatch(new RegExp(`font-weight: ${weight};`));
});

// A rule that writes the property itself outranks every cut set around it.
it('draws every cut through one rule that reads the two axes', () => {
  const writers = rules
    .filter(([, , body]) => /font-variation-settings:/.test(body))
    .map(([path, selector, body]) => [path, selector, body.replace(/\s+/g, ' ').trim()]);
  expect(writers).toEqual([
    [
      'tokens.css',
      '*,\nbutton,\ninput,\nselect,\ntextarea',
      "font-variation-settings: 'wght' var(--wght), 'wdth' var(--wdth);",
    ],
  ]);
});

it('names the note cut once, for light and dark alike', () => {
  expect(tokens.match(/--cut-note-weight:/g)).toEqual(['--cut-note-weight:']);
  expect(tokens.match(/--cut-note-width:/g)).toEqual(['--cut-note-width:']);
  expect(tokens).toMatch(/--cut-note-weight: 420;/);
  expect(tokens).toMatch(/--cut-note-width: 90;/);
});

const noteType = () =>
  rules.find(
    ([path, named]) => path === 'tokens.css' && named.split(/,\s*/).includes('.note-type'),
  );

it('sets the note type in the note cut, at its line height and tracking, with no colour or measure', () => {
  const [, selector, body] = noteType() ?? ['', '', ''];
  expect(selector.split(/,\s*/)).toContain('.note');
  expect(body).toMatch(/--wght: var\(--cut-note-weight\);\s*--wdth: var\(--cut-note-width\);/);
  expect(body).toMatch(/font-size: 13px;/);
  expect(body).toMatch(/line-height: 1\.5;/);
  expect(body).toMatch(/letter-spacing: 0\.005em;/);
  expect(body).not.toMatch(/color:|max-width:/);
});

it('holds a note to its measure', () => {
  expect(ruleFor('.note')).toMatch(/max-width: 440px;/);
});

it('sets no small agent cut by hand: 13px agent text is a note', () => {
  const offenders = rules
    .filter(([, , body]) => /font-size: 13px;/.test(body) && /--cut-agent-weight/.test(body))
    .map(([path, selector]) => `${path} ${selector}`);
  expect(offenders).toEqual([]);
});

// A copy drifts from the role it was taken from.
it('states the note cut in the note type role alone', () => {
  const copies = rules
    .filter(([, , body]) => /var\(--cut-note-weight\)/.test(body))
    .map(([path, selector]) => `${path} ${selector}`);
  expect(copies).toEqual([`tokens.css ${noteType()?.[1]}`]);
});

// jsdom lays nothing out, so the shared column is read from the stylesheet.
it('starts every choice in a list on one edge, with a radio or none', () => {
  expect(ruleFor('.choice-row')).toMatch(/grid-template-columns: var\(--mark\) minmax\(0, 1fr\);/);
  expect(ruleFor('.choice-row > *')).toMatch(/grid-column: 2;/);
  expect(ruleFor('.choice-row > :first-child')).toMatch(/grid-column: 1;/);
  const ownSpacers = rules
    .filter(([, selector, body]) => /::before/.test(selector) && /width:/.test(body))
    .filter(([path]) => /(Speaking|Listening)Page/.test(path));
  expect(ownSpacers).toEqual([]);
});

// A global role grids whatever else carries its class, so no component names it.
it('keeps the choice row to its own name', () => {
  const claims = rules
    .filter(([path, selector]) => path !== 'tokens.css' && /\.choice-row\b/.test(selector))
    .map(([path, selector]) => `${path} ${selector}`);
  expect(claims).toEqual([]);
});

// KeyRow names its own visible line `.held`, and a global rule reaches inside every component.
it('hides the spare label only inside the button that holds it', () => {
  expect(ruleFor('.holds > .held')).toMatch(/visibility: hidden;/);
  expect(ruleFor('.held')).toBe('');
});
