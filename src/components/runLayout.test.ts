/**
 * The claim: the run column splits only on a width that was actually measured
 * and is actually wide.
 *
 * Both failure directions cost something real. Splitting too eagerly — or on
 * the zeros a hidden, not-yet-laid-out column reports — renders two cramped
 * tracks where one readable one belongs, and `null` is the state every mount
 * starts in. Never splitting wastes the wide window this feature exists to
 * fill. So the threshold is pinned on both sides of `SPLIT_MIN_WIDTH` here,
 * rather than left to be eyeballed by resizing a 4K window.
 */
import { describe, expect, it } from 'vitest';

import { pickRunLayout, PROSE_CH, SPLIT_MIN_WIDTH } from './runLayout';

describe('pickRunLayout', () => {
  it('stacks before anything has been measured', () => {
    expect(pickRunLayout(null)).toBe('stacked');
  });

  it('stacks on a laptop-width column', () => {
    expect(pickRunLayout({ width: 1100, height: 620 })).toBe('stacked');
  });

  it('stacks one pixel below the threshold', () => {
    expect(pickRunLayout({ width: SPLIT_MIN_WIDTH - 1, height: 900 })).toBe('stacked');
  });

  it('splits exactly at the threshold', () => {
    expect(pickRunLayout({ width: SPLIT_MIN_WIDTH, height: 900 })).toBe('split');
  });

  it('splits on a 4K column', () => {
    expect(pickRunLayout({ width: 3400, height: 1600 })).toBe('split');
  });

  it('stacks on a zero width', () => {
    expect(pickRunLayout({ width: 0, height: 900 })).toBe('stacked');
  });

  it('stacks on a negative width', () => {
    expect(pickRunLayout({ width: -1600, height: 900 })).toBe('stacked');
  });

  it('stacks on a zero height, however wide', () => {
    expect(pickRunLayout({ width: 3400, height: 0 })).toBe('stacked');
  });
});

describe('constants', () => {
  it('pins the split threshold and the prose measure', () => {
    expect(SPLIT_MIN_WIDTH).toBe(1600);
    expect(PROSE_CH).toBe(96);
  });
});
