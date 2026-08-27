import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import type { LampForm } from '../lib/daemon';
import Lamp from './Lamp.svelte';
it('draws four distinct forms', () => {
  const html = (form: LampForm) => render(Lamp, { form }).container.innerHTML;
  const forms = (['idle', 'recording', 'speaking', 'notrunning'] satisfies LampForm[]).map(html);
  expect(new Set(forms).size).toBe(4);
  expect(html('notrunning')).toContain('stroke-dasharray');
  expect(html('speaking')).toContain('M8 40');
});
it('colours only the recording form live', () => {
  const html = (form: LampForm) => render(Lamp, { form }).container.innerHTML;
  expect(html('recording')).toContain('var(--live)');
  expect(html('idle')).not.toContain('var(--live)');
  expect(html('speaking')).not.toContain('var(--live)');
  expect(html('notrunning')).not.toContain('var(--live)');
});
