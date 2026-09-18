// Which browser code carries which key is a browser fact, so it lives here.
// Which of them bind is the daemon's, and arrives as `bindable_modifiers`:
// the set differs per platform and a table of our own drifted from it.
const NAMES: Record<string, string> = {
  AltRight: 'RightOption',
  AltLeft: 'LeftOption',
  ControlLeft: 'LeftControl',
  ControlRight: 'RightControl',
  MetaLeft: 'LeftCommand',
  MetaRight: 'RightCommand',
};

function named(code: string, bindable: readonly string[]): string | null {
  const name = NAMES[code];
  return name !== undefined && bindable.includes(name) ? name : null;
}

// A modifier can be the whole binding or the head of a chord. Only its
// release tells which, so a caller waits before it commits one.
export function isModifier(code: string, bindable: readonly string[]): boolean {
  return named(code, bindable) !== null;
}

// The daemon reads a hotkey with no spaces, and a reader needs them.
export function humanize(hotkey: string): string {
  return hotkey
    .split('+')
    .map((part) => part.replace(/([a-z0-9])([A-Z])/g, '$1 $2'))
    .join(' + ');
}

export function hotkeyFrom(
  event: {
    code: string;
    ctrlKey: boolean;
    altKey: boolean;
    metaKey: boolean;
    shiftKey?: boolean;
  },
  bindable: readonly string[],
): string | null {
  // The daemon reserves every Shift form, and a chord that silently drops it
  // would bind a key the user never pressed.
  if (event.shiftKey === true) return null;
  const modifier = named(event.code, bindable);
  if (modifier !== null) return modifier;
  // A modifier the daemon refuses is not a main key either.
  if (event.code in NAMES) return null;

  const main = /^F([1-9]|1[0-2])$/.test(event.code)
    ? event.code
    : /^Key[A-Z]$/.test(event.code)
      ? event.code.slice(3)
      : /^Digit[0-9]$/.test(event.code)
        ? event.code.slice(5)
        : null;
  if (main === null) return null;

  const chord = [
    event.ctrlKey ? 'Ctrl' : '',
    event.altKey ? 'Alt' : '',
    event.metaKey ? 'Cmd' : '',
  ].filter(Boolean);
  return [...chord, main].join('+');
}
