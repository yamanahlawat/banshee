import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { expect, it, vi } from 'vitest';
import Failure from './Failure.svelte';
import { forgetCopy } from '../lib/copy';

vi.mock('../lib/tauri', () => ({ copyText: vi.fn().mockResolvedValue(null) }));

function failure(said: string) {
  return render(Failure, {
    id: 'speech-failure',
    label: 'The speaker failed.',
    said,
  });
}

// A panel keeps one id for its failure row, so the confirmation has to follow
// the text. A second failure inside the held 1500 ms would otherwise leave
// Copied standing over words nobody copied.
it('drops the confirmation when the failure text changes', async () => {
  forgetCopy();

  const { getByRole, rerender } = failure('The server refused the key.');
  const said = () => getByRole('button').textContent ?? '';
  await fireEvent.click(getByRole('button'));
  expect(said()).toContain('Copied');

  await rerender({ said: 'The server closed the connection.' });
  await tick();
  expect(said()).not.toContain('Copied');
  expect(said()).toContain('Copy');

  forgetCopy();
});
