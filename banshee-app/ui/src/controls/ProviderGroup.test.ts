import { fireEvent, render } from '@testing-library/svelte';
import { expect, it, vi } from 'vitest';
import ProviderGroup from './ProviderGroup.svelte';

const OPTIONS = [
  { value: 'local', label: 'On this machine' },
  { value: 'remote', label: 'A remote server' },
];

function group(props: Record<string, unknown> = {}) {
  return render(ProviderGroup, {
    name: 'Listening',
    label: 'Listening',
    value: 'remote',
    options: OPTIONS,
    note: 'Audio goes to api.openai.com.',
    noteId: 'listener-note',
    change: () => {},
    ...props,
  });
}

// The choice and its consequence are one reading, so the radiogroup names the
// sentence that says what the choice does.
it('describes the choice by its note', () => {
  const { getByRole } = group();
  const described = getByRole('radiogroup', { name: 'Listening' }).getAttribute('aria-describedby');
  expect(described).toBe('listener-note');
  expect(document.getElementById('listener-note')?.textContent).toBe(
    'Audio goes to api.openai.com.',
  );
});

it('hands the chosen value back', async () => {
  const change = vi.fn();
  const { getByRole } = group({ change });
  await fireEvent.click(getByRole('radio', { name: 'On this machine' }));
  expect(change).toHaveBeenCalledWith('local');
});
