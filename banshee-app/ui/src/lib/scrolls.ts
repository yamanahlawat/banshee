/// Gives a box a tab stop while its content overflows it: a keyboard cannot
/// scroll a box it cannot reach, and a box with nothing to scroll is one more
/// stop for nothing.
export function tabWhenScrolls(node: HTMLElement) {
  const check = () => {
    if (node.scrollHeight > node.clientHeight) node.tabIndex = 0;
    else node.removeAttribute('tabindex');
  };
  check();
  const content = new MutationObserver(check);
  content.observe(node, { childList: true, subtree: true, characterData: true });
  const size = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(check);
  size?.observe(node);
  return {
    destroy() {
      content.disconnect();
      size?.disconnect();
    },
  };
}
