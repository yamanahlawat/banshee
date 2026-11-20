import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Absence from './Absence.svelte';
import source from './Absence.svelte?raw';

const box = (name: string) => screen.getByRole('heading', { name }).parentElement as HTMLElement;

describe('Absence', () => {
  it('keeps the dashed box by default', () => {
    render(Absence, { label: 'Not running' });
    expect(box('Not running').classList.contains('plain')).toBe(false);
    expect(source).toMatch(/\.absence\s*\{[^}]*border:\s*1px dashed var\(--accent\)/);
  });

  it('drops the edge and the side padding when plain', () => {
    render(Absence, { label: 'Nothing here yet', plain: true });
    expect(box('Nothing here yet').classList.contains('plain')).toBe(true);
    expect(source).toMatch(/\.plain\s*\{[^}]*border:\s*none[^}]*padding:\s*0/);
  });
});
