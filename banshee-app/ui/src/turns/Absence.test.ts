import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Absence from './Absence.svelte';

// The indent exists to line the box up with the text column of the turns
// beside it. A box that stands above the record has no turns to line up with,
// only bands at the gutter they all share.
it('sits at the band gutter, like the bands it stands among', () => {
  const { container } = render(Absence, { label: 'Banshee is not running' });
  expect(container.querySelector('.absence')?.classList.contains('in-record')).toBe(false);
});

it('takes the turn text column when it stands in for a turn', () => {
  const { container } = render(Absence, { label: 'Nothing said yet', inRecord: true });
  expect(container.querySelector('.absence')?.classList.contains('in-record')).toBe(true);
});
