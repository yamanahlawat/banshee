import { afterEach, describe, expect, it, vi } from 'vitest';
import { tabWhenScrolls } from './scrolls';

function box(content: number, height: number): HTMLElement {
  const node = document.createElement('div');
  Object.defineProperty(node, 'scrollHeight', { get: () => content, configurable: true });
  Object.defineProperty(node, 'clientHeight', { get: () => height, configurable: true });
  document.body.append(node);
  return node;
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('tabWhenScrolls', () => {
  it('gives a box that overflows a tab stop', () => {
    const node = box(400, 200);
    tabWhenScrolls(node);
    expect(node.getAttribute('tabindex')).toBe('0');
  });

  it('gives a box that holds its content no tab stop', () => {
    const node = box(200, 200);
    tabWhenScrolls(node);
    expect(node.hasAttribute('tabindex')).toBe(false);
  });

  it('takes the stop when added content overflows the box, and drops it when it goes', async () => {
    let content = 100;
    const node = box(0, 200);
    Object.defineProperty(node, 'scrollHeight', { get: () => content });
    const { destroy } = tabWhenScrolls(node);

    content = 400;
    node.append(document.createElement('p'));
    await Promise.resolve();
    expect(node.getAttribute('tabindex')).toBe('0');

    content = 100;
    node.replaceChildren();
    await Promise.resolve();
    expect(node.hasAttribute('tabindex')).toBe(false);
    destroy();
  });
});
