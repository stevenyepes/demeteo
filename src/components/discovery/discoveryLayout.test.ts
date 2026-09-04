/**
 * The claim: the discovery workspace row picks its column count from a width
 * that was actually measured and is actually wide enough to seat them.
 *
 * Both failure directions cost something real. Splitting into three too
 * eagerly — or on the zeros a hidden, not-yet-laid-out row reports — wedges
 * the graph or the inspector below the width it needs to be read. Never
 * widening past `'stacked'` wastes the room a real window has. So each
 * threshold is pinned on both sides here, rather than left to be eyeballed by
 * resizing a window by hand.
 */
import { describe, expect, it } from 'vitest';

import {
  OVERLAY_MIN_WIDTH,
  pickDiscoveryLayout,
  THREE_UP_MIN_WIDTH,
} from './discoveryLayout';

describe('pickDiscoveryLayout', () => {
  it('stacks before anything has been measured', () => {
    expect(pickDiscoveryLayout(null)).toBe('stacked');
  });

  it('stacks on a zero width', () => {
    expect(pickDiscoveryLayout({ width: 0, height: 900 })).toBe('stacked');
  });

  it('stacks on a negative width', () => {
    expect(pickDiscoveryLayout({ width: -920, height: 900 })).toBe('stacked');
  });

  it('stacks on a zero height, however wide', () => {
    expect(pickDiscoveryLayout({ width: 3400, height: 0 })).toBe('stacked');
  });

  it('stacks on a negative height, however wide', () => {
    expect(pickDiscoveryLayout({ width: 3400, height: -1 })).toBe('stacked');
  });

  it('stacks one pixel below the overlay threshold', () => {
    expect(pickDiscoveryLayout({ width: OVERLAY_MIN_WIDTH - 1, height: 900 })).toBe('stacked');
  });

  it('goes overlay-inspector exactly at the overlay threshold', () => {
    expect(pickDiscoveryLayout({ width: OVERLAY_MIN_WIDTH, height: 900 })).toBe(
      'overlay-inspector',
    );
  });

  it('stays overlay-inspector one pixel below the three-up threshold', () => {
    expect(pickDiscoveryLayout({ width: THREE_UP_MIN_WIDTH - 1, height: 900 })).toBe(
      'overlay-inspector',
    );
  });

  it('goes three-up exactly at the three-up threshold', () => {
    expect(pickDiscoveryLayout({ width: THREE_UP_MIN_WIDTH, height: 900 })).toBe('three-up');
  });

  it('stays three-up on a wide 4K row', () => {
    expect(pickDiscoveryLayout({ width: 3400, height: 1600 })).toBe('three-up');
  });
});
