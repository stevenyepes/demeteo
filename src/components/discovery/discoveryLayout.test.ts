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
  COLLAPSED_RAIL_WIDTH,
  GRAPH_MIN_WIDTH,
  INSPECTOR_WIDTH,
  OVERLAY_MIN_WIDTH,
  pickDiscoveryLayout,
  RECLAIMED_BY_HIDING,
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

  describe('with the interview hidden', () => {
    it('seats the inspector in flow on a row that could only overlay it', () => {
      const width = THREE_UP_MIN_WIDTH - 1;
      expect(pickDiscoveryLayout({ width, height: 900 })).toBe('overlay-inspector');
      expect(pickDiscoveryLayout({ width, height: 900 }, true)).toBe('three-up');
    });

    it('still overlays one pixel below the reclaimed three-up threshold', () => {
      const width = THREE_UP_MIN_WIDTH - RECLAIMED_BY_HIDING - 1;
      expect(pickDiscoveryLayout({ width, height: 900 }, true)).toBe('overlay-inspector');
    });

    it('lifts a stacked row to overlay-inspector at the reclaimed threshold', () => {
      const width = OVERLAY_MIN_WIDTH - RECLAIMED_BY_HIDING;
      expect(pickDiscoveryLayout({ width, height: 900 })).toBe('stacked');
      expect(pickDiscoveryLayout({ width, height: 900 }, true)).toBe('overlay-inspector');
    });

    it('still stacks below the graph minimum, where there is nothing to reclaim into', () => {
      const width = OVERLAY_MIN_WIDTH - RECLAIMED_BY_HIDING - 1;
      expect(pickDiscoveryLayout({ width, height: 900 }, true)).toBe('stacked');
    });

    // The bug this pins: the rail that replaces the interview is not free, so
    // reclaiming the whole column overdraws each threshold by its width and
    // leaves the graph under its minimum — which `TicketColumn` absorbs
    // silently, being `flex-1 min-w-0`. Widths, not modes: a mode assertion
    // reads as a pass at any threshold, however wrong.
    it('leaves every pane its minimum at the width each reclaimed threshold admits', () => {
      const overlayRow = OVERLAY_MIN_WIDTH - RECLAIMED_BY_HIDING;
      expect(overlayRow - COLLAPSED_RAIL_WIDTH).toBeGreaterThanOrEqual(GRAPH_MIN_WIDTH);

      const threeUpRow = THREE_UP_MIN_WIDTH - RECLAIMED_BY_HIDING;
      expect(threeUpRow - COLLAPSED_RAIL_WIDTH - INSPECTOR_WIDTH).toBeGreaterThanOrEqual(
        GRAPH_MIN_WIDTH,
      );
    });

    it('reclaims nothing from an unmeasured row', () => {
      expect(pickDiscoveryLayout(null, true)).toBe('stacked');
      expect(pickDiscoveryLayout({ width: 0, height: 0 }, true)).toBe('stacked');
    });
  });
});
