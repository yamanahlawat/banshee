import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Mark from './Mark.svelte';

it('draws listening as two solid ellipses, not a bar', () => {
  const { container } = render(Mark, { form: 'listening' });
  const ellipses = container.querySelectorAll('ellipse');
  expect(ellipses.length).toBe(2);
  expect(container.querySelector('rect[width="34"]')).toBeNull();
});

it('draws busy as a ring behind the head, with its near half in front', () => {
  const { container } = render(Mark, { form: 'busy' });
  const ring = container.querySelectorAll('path[fill-rule="evenodd"]');
  expect(ring.length).toBe(2);
  expect(ring[0].getAttribute('mask')).toBe('url(#mark-busy-mask)');
  expect(ring[1].getAttribute('clip-path')).toBe('url(#mark-busy-near)');
  expect(ring[0].getAttribute('stroke')).toBeNull();
});

it('draws neither the headphones nor the ring for idle', () => {
  const { container } = render(Mark, { form: 'idle' });
  expect(container.querySelectorAll('ellipse').length).toBe(0);
  expect(container.querySelectorAll('path[fill-rule="evenodd"]').length).toBe(0);
});

it('leaves recording as the filled shroud alone', () => {
  const { container } = render(Mark, { form: 'recording' });
  expect(container.querySelector('path')?.getAttribute('fill')).toBe('var(--accent)');
  expect(container.querySelectorAll('path[fill-rule="evenodd"]').length).toBe(0);
});
