// The window's own key handlers are registered first and would otherwise win,
// so a panel capturing keystrokes has to claim them.
let held = false;

// Releasing twice is a no-op, so a component may release in a handler and
// again in its teardown.
export function claimKeys(): () => void {
  held = true;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    held = false;
  };
}

export function keysClaimed(): boolean {
  return held;
}

// Module state outlives a test, so the suite resets this beside the stores.
export function forgetKeys(): void {
  held = false;
}
