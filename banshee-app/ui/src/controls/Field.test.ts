import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Field from './Field.svelte';

// A row still showing the refused text reads as a value the daemon took, and
// the toast beside it says the opposite.
it('goes back to the value the daemon holds when the write is refused', async () => {
  render(Field, { label: 'Server', value: 'https://api.openai.com/v1', commit: () => false });
  const input = screen.getByRole('textbox', { name: 'Server' }) as HTMLInputElement;
  await fireEvent.input(input, { target: { value: '' } });
  await fireEvent.blur(input);
  await waitFor(() => expect(input.value).toBe('https://api.openai.com/v1'));
});

it('commits on Enter and on blur, once per change', async () => {
  const committed: string[] = [];
  render(Field, {
    label: 'Server',
    value: 'a',
    commit: (next: string) => void committed.push(next),
  });
  const input = screen.getByRole('textbox', { name: 'Server' }) as HTMLInputElement;
  await fireEvent.input(input, { target: { value: 'b' } });
  await fireEvent.keyDown(input, { key: 'Enter' });
  expect(committed).toEqual(['b']);
  await fireEvent.blur(input);
  expect(committed).toEqual(['b']);
});

it('commits on blur alone, and once', async () => {
  const committed: string[] = [];
  render(Field, {
    label: 'Server',
    value: 'a',
    commit: (next: string) => void committed.push(next),
  });
  const input = screen.getByRole('textbox', { name: 'Server' }) as HTMLInputElement;
  await fireEvent.focus(input);
  await fireEvent.input(input, { target: { value: 'b' } });
  await fireEvent.blur(input);
  expect(committed).toEqual(['b']);
  await fireEvent.focus(input);
  await fireEvent.blur(input);
  expect(committed).toEqual(['b']);
});

// The daemon takes a key and never gives it back, so the field has nothing to hold.
it('commits a masked value once and then shows its placeholder again', async () => {
  const committed: string[] = [];
  const { container } = render(Field, {
    label: 'Key',
    masked: true,
    placeholder: 'Not set',
    commit: (next: string) => void committed.push(next),
  });
  const input = container.querySelector('input') as HTMLInputElement;
  await fireEvent.focus(input);
  await fireEvent.input(input, { target: { value: 'sk-a-secret' } });
  await fireEvent.blur(input);
  expect(committed).toEqual(['sk-a-secret']);
  expect(input.value).toBe('');
  await fireEvent.focus(input);
  await fireEvent.blur(input);
  expect(committed).toEqual(['sk-a-secret']);
});

it('reverts on Escape and commits nothing', async () => {
  const committed: string[] = [];
  render(Field, {
    label: 'Model',
    value: 'whisper-1',
    commit: (next: string) => void committed.push(next),
  });
  const input = screen.getByRole('textbox', { name: 'Model' }) as HTMLInputElement;
  await fireEvent.input(input, { target: { value: 'typo' } });
  await fireEvent.keyDown(input, { key: 'Escape' });
  expect(input.value).toBe('whisper-1');
  expect(committed).toEqual([]);
});

it('masks its input and starts from its placeholder, not a value', () => {
  const { container } = render(Field, {
    label: 'Key',
    masked: true,
    placeholder: 'Not set',
    commit: () => {},
  });
  const input = container.querySelector('input') as HTMLInputElement;
  expect(input.type).toBe('password');
  expect(input.value).toBe('');
  expect(input.placeholder).toBe('Not set');
});
