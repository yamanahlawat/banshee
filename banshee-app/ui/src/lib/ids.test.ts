import { expect, it } from 'vitest';
import { idList } from './ids';

it('lists the ids that apply, in order', () => {
  expect(idList('note-a', false, undefined, 'refused-b', null)).toBe('note-a refused-b');
});

it('gives nothing rather than an empty list', () => {
  expect(idList(false, undefined, null, '')).toBeUndefined();
  expect(idList()).toBeUndefined();
});
