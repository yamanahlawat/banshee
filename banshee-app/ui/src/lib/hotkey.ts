// The daemon's own name for each modifier. Whether it binds one is its own
// answer and turns on the platform, so a name here is not a promise.
const MODIFIERS: Record<string, string> = {
  AltRight: 'RightOption',
  AltLeft: 'LeftOption',
  ControlLeft: 'LeftControl',
  ControlRight: 'RightControl',
  MetaLeft: 'LeftCommand',
  MetaRight: 'RightCommand',
};

// A modifier can be the whole binding or the head of a chord. Only its
// release tells which, so a caller waits before it commits one.
export function isModifier(code: string): boolean {
  return code in MODIFIERS;
}

// The daemon reads a hotkey with no spaces, and a reader needs them.
export function humanize(hotkey: string): string {
  return hotkey
    .split('+')
    .map((part) => part.replace(/([a-z0-9])([A-Z])/g, '$1 $2'))
    .join(' + ');
}

// A press becomes the daemon's grammar: a chord, an F-key, a lone modifier,
// or one typed character. `null` for a press it can never bind.
export function hotkeyFrom(event: {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  shiftKey?: boolean;
}): string | null {
  // The daemon reserves every Shift form, and a chord that silently drops it
  // would bind a key the user never pressed.
  if (event.shiftKey === true) return null;
  const modifier = MODIFIERS[event.code];
  if (modifier) return modifier;

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
