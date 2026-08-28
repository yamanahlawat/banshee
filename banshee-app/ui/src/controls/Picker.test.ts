import { render, screen } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Picker from './Picker.svelte';

const devices = [
  { value: 'MacBook Pro Microphone', label: 'MacBook Pro Microphone' },
  { value: 'Yeti', label: 'Yeti' },
];

it('shows the value the daemon holds even when no option carries it', () => {
  render(Picker, { options: devices, value: 'Yeti X', label: 'Input', change: () => {} });
  const select = screen.getByLabelText('Input') as HTMLSelectElement;
  expect(select.value).toBe('Yeti X');
  expect(screen.getByRole('option', { name: 'Yeti X' })).toBeTruthy();
});

it('adds no option when one already carries the value', () => {
  render(Picker, { options: devices, value: 'Yeti', label: 'Input', change: () => {} });
  expect(screen.getAllByRole('option')).toHaveLength(2);
});
