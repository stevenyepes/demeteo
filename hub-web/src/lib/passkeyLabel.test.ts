import { describe, expect, it } from 'vitest';

import { isUsablePasskeyLabel } from './passkeyLabel';

describe('isUsablePasskeyLabel', () => {
  it.each([
    ['U+0000', 'a\u0000b'],
    ['a newline', 'line\nbreak'],
    ['DEL', 'a\u007fb'],
  ])('refuses a label containing %s', (_name, label) => {
    expect(isUsablePasskeyLabel(label)).toBe(false);
  });

  it('counts an emoji as one character of the 64', () => {
    expect(isUsablePasskeyLabel('📱'.repeat(64))).toBe(true);
    expect(isUsablePasskeyLabel('📱'.repeat(65))).toBe(false);
  });

  it('refuses an empty label', () => {
    expect(isUsablePasskeyLabel('')).toBe(false);
  });
});
