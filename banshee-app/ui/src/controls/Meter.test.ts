import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Meter from './Meter.svelte';

it('clamps aria-valuenow to its declared bounds', () => {
  const meter = (level: number) => render(Meter, { level, live: false }).container.querySelector('[role="meter"]');
  expect(meter(140)?.getAttribute('aria-valuenow')).toBe('100');
  expect(meter(-20)?.getAttribute('aria-valuenow')).toBe('0');
});
