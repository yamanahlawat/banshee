import { describe, expect, it } from 'vitest';
import { hotkeyFrom, humanize, isModifier } from './hotkey';

const press = (
  code: string,
  held: Partial<{ ctrlKey: boolean; altKey: boolean; metaKey: boolean; shiftKey: boolean }> = {},
) => ({
  code,
  ctrlKey: false,
  altKey: false,
  metaKey: false,
  // A browser reports this on the Shift press itself. Without it a Shift press
  // reaches the name regexes rather than the guard that refuses it.
  shiftKey: code.startsWith('Shift'),
  ...held,
});

// What `banshee status` reports on each platform, from the one table in
// bansheed/src/binding.rs.
const ON_LINUX = ['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightControl'];
const ON_MACOS = ['RightOption', 'LeftOption', 'LeftControl', 'LeftCommand', 'RightCommand', 'Fn'];

describe('the modifiers the daemon reports', () => {
  it('offers Right Control on Linux, which the daemon binds there', () => {
    expect(hotkeyFrom(press('ControlRight'), ON_LINUX)).toBe('RightControl');
    expect(isModifier('ControlRight', ON_LINUX)).toBe(true);
  });

  it('refuses Right Command on Linux, which the daemon cannot bind there', () => {
    expect(hotkeyFrom(press('MetaRight'), ON_LINUX)).toBeNull();
    expect(isModifier('MetaRight', ON_LINUX)).toBe(false);
  });

  it('turns the two around on macOS', () => {
    expect(hotkeyFrom(press('MetaRight'), ON_MACOS)).toBe('RightCommand');
    expect(hotkeyFrom(press('ControlRight'), ON_MACOS)).toBeNull();
  });

  it('refuses every modifier when the daemon has reported none yet', () => {
    expect(hotkeyFrom(press('AltRight'), [])).toBeNull();
    expect(isModifier('AltRight', [])).toBe(false);
  });
});

describe('hotkeyFrom', () => {
  it('names a lone modifier the way the daemon does', () => {
    expect(hotkeyFrom(press('AltRight'), ON_LINUX)).toBe('RightOption');
    expect(hotkeyFrom(press('MetaRight'), ON_MACOS)).toBe('RightCommand');
  });
  it('keeps a lone modifier lone, whatever the browser reports as held', () => {
    // The browser marks the modifier itself as down during its own press.
    expect(hotkeyFrom(press('AltRight', { altKey: true }), ON_LINUX)).toBe('RightOption');
  });
  it('takes an F-key as it stands', () => {
    expect(hotkeyFrom(press('F6'), ON_LINUX)).toBe('F6');
  });
  it('refuses an F-key the daemon cannot receive', () => {
    expect(hotkeyFrom(press('F13'), ON_LINUX)).toBeNull();
  });
  it("builds a chord in the daemon's order", () => {
    expect(hotkeyFrom(press('KeyR', { ctrlKey: true, altKey: true }), ON_LINUX)).toBe('Ctrl+Alt+R');
  });
  it('refuses Shift, which the daemon reserves', () => {
    expect(hotkeyFrom(press('ShiftLeft'), ON_LINUX)).toBeNull();
    // The rule is the chord, not the lone key, and a lone modifier alone would
    // pass here on its name rather than on the guard.
    expect(hotkeyFrom(press('KeyR', { shiftKey: true }), ON_LINUX)).toBeNull();
    expect(hotkeyFrom(press('F6', { shiftKey: true }), ON_LINUX)).toBeNull();
  });
  it('refuses a key it cannot name', () => {
    expect(hotkeyFrom(press('CapsLock'), ON_LINUX)).toBeNull();
  });
});

describe('humanize', () => {
  it("spaces the daemon's run-together name", () => {
    expect(humanize('RightOption')).toBe('Right Option');
  });
  it('spaces every part of a chord', () => {
    expect(humanize('Ctrl+Alt+R')).toBe('Ctrl + Alt + R');
  });
});

it('refuses a chord carrying Shift rather than writing a different one', () => {
  // Ctrl+Shift+D must never be written as Ctrl+D.
  expect(hotkeyFrom({ ...press('KeyD', { ctrlKey: true }), shiftKey: true }, ON_LINUX)).toBeNull();
});
