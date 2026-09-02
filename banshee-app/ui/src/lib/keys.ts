/// Where an arrow moves inside a one-tab-stop group; null when the key is not an arrow.
export function arrowStep(key: string, at: number, count: number): number | null {
  const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[key];
  if (step === undefined || count === 0) return null;
  // A value the group does not hold leaves `at` at -1, and the first cell is
  // the right place to start from.
  return (at + step + count) % count;
}

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
