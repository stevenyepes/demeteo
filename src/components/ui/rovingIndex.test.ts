import { describe, expect, it } from 'vitest';

import { nextIndexForKey } from './rovingIndex';

describe('nextIndexForKey', () => {
  it('moves one step in either direction', () => {
    expect(nextIndexForKey('ArrowRight', 0, 3)).toBe(1);
    expect(nextIndexForKey('ArrowLeft', 2, 3)).toBe(1);
  });

  it('treats the vertical arrows as the horizontal ones', () => {
    expect(nextIndexForKey('ArrowDown', 0, 3)).toBe(1);
    expect(nextIndexForKey('ArrowUp', 1, 3)).toBe(0);
  });

  it('wraps at both ends', () => {
    expect(nextIndexForKey('ArrowRight', 2, 3)).toBe(0);
    expect(nextIndexForKey('ArrowLeft', 0, 3)).toBe(2);
  });

  it('jumps to the ends with Home and End', () => {
    expect(nextIndexForKey('Home', 2, 3)).toBe(0);
    expect(nextIndexForKey('End', 0, 3)).toBe(2);
  });

  it('starts from the first entry when nothing is selected', () => {
    expect(nextIndexForKey('ArrowRight', -1, 3)).toBe(1);
    expect(nextIndexForKey('ArrowLeft', -1, 3)).toBe(2);
  });

  it('claims no other key', () => {
    expect(nextIndexForKey('a', 0, 3)).toBeNull();
    expect(nextIndexForKey('Enter', 0, 3)).toBeNull();
    expect(nextIndexForKey(' ', 0, 3)).toBeNull();
    expect(nextIndexForKey('Tab', 0, 3)).toBeNull();
  });

  it('claims nothing at all in an empty row', () => {
    expect(nextIndexForKey('ArrowRight', 0, 0)).toBeNull();
    expect(nextIndexForKey('Home', 0, 0)).toBeNull();
    expect(nextIndexForKey('End', -1, 0)).toBeNull();
  });
});
