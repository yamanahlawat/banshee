import { expect, it } from 'vitest';
import source from './Find.svelte?raw';

it('leaves the focus ring to the search wrapper', () => {
  const style = source.slice(source.indexOf('<style>'));
  expect(style).toMatch(/\.search:focus-within\s*\{[^}]*outline:\s*2px solid/);
  expect(style).toMatch(/\.search input:focus-visible\s*\{[^}]*outline:\s*none/);
});
