import { describe, expect, it } from 'vitest';
import { hotkeyFrom, humanize } from './hotkey';

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

describe('hotkeyFrom', () => {
  it('names a lone modifier the way the daemon does', () => {
    expect(hotkeyFrom(press('AltRight'))).toBe('RightOption');
    expect(hotkeyFrom(press('MetaRight'))).toBe('RightCommand');
  });
  it('keeps a lone modifier lone, whatever the browser reports as held', () => {
    // The browser marks the modifier itself as down during its own press.
    expect(hotkeyFrom(press('AltRight', { altKey: true }))).toBe('RightOption');
  });
  it('takes an F-key as it stands', () => {
    expect(hotkeyFrom(press('F6'))).toBe('F6');
  });
  it('refuses an F-key the daemon cannot receive', () => {
    expect(hotkeyFrom(press('F13'))).toBeNull();
  });
  it("builds a chord in the daemon's order", () => {
    expect(hotkeyFrom(press('KeyR', { ctrlKey: true, altKey: true }))).toBe('Ctrl+Alt+R');
  });
  it('refuses Shift, which the daemon reserves', () => {
    expect(hotkeyFrom(press('ShiftLeft'))).toBeNull();
    // The rule is the chord, not the lone key, and a lone modifier alone would
    // pass here on its name rather than on the guard.
    expect(hotkeyFrom(press('KeyR', { shiftKey: true }))).toBeNull();
    expect(hotkeyFrom(press('F6', { shiftKey: true }))).toBeNull();
  });
  it('refuses a key it cannot name', () => {
    expect(hotkeyFrom(press('CapsLock'))).toBeNull();
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
  expect(hotkeyFrom({ ...press('KeyD', { ctrlKey: true }), shiftKey: true })).toBeNull();
});
