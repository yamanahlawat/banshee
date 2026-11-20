import { expect } from 'vitest';

/// Each node follows the one before it in the document.
export function expectInOrder(...nodes: Node[]) {
  for (let at = 1; at < nodes.length; at++) {
    expect(
      nodes[at - 1].compareDocumentPosition(nodes[at]) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  }
}
