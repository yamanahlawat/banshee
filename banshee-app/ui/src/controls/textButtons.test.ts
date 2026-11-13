import { expect, it } from 'vitest';

const sources = import.meta.glob('../**/*.svelte', {
  query: '?raw',
  import: 'default',
  eager: true,
});

it('no text button carries the caps class', () => {
  const offenders = Object.entries(sources)
    .filter(([, source]) =>
      [...(source as string).matchAll(/class="([^"]*)"/g)].some(
        ([, list]) => /\bcaps\b/.test(list) && /\bbtn-underline\b/.test(list),
      ),
    )
    .map(([path]) => path);
  expect(offenders).toEqual([]);
});
