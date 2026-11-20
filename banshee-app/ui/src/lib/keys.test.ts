import { describe, expect, it } from 'vitest';
import { onMac } from './keys';

const MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
const LINUX = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15';

describe('onMac', () => {
  it('is true on macOS', () => {
    expect(onMac(MAC)).toBe(true);
  });

  // A Linux keyboard has no Command key, so its shortcuts take Control.
  it('is false everywhere else', () => {
    expect(onMac(LINUX)).toBe(false);
  });
});
