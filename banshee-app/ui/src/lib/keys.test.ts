import { describe, expect, it } from 'vitest';
import { findChord } from './keys';

const MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
const LINUX = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15';

describe('findChord', () => {
  it('draws the Command glyph on macOS', () => {
    expect(findChord(MAC)).toBe('⌘F');
  });

  // A Linux keyboard has no Command key, so the glyph names a key nobody has.
  it('names Control everywhere else', () => {
    expect(findChord(LINUX)).toBe('Ctrl+F');
  });
});
