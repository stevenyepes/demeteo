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

import {
  defaultInspectorWidth,
  INSPECTOR_SIDE_MIN_WIDTH,
  INSPECTOR_WIDTH_FRACTION,
  pickInspectorLayout,
  pickRunLayout,
  PROSE_CH,
  SPLIT_MIN_WIDTH,
} from './runLayout';
import { DEFAULT_MIN_PRIMARY, DEFAULT_MIN_SECONDARY } from './ui/splitPaneGeometry';

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

/**
 * The claim: the inspector is on screen at every column width, and side-by-side
 * only where both panes fit at their minimums.
 *
 * The failure this pins is not cosmetic. The settled decision is that the
 * inspector never collapses to zero, so a column below `minPrimary +
 * minSecondary` that still forces a split leaves both panes pinned at their
 * minimums with the run surface unusable and no way out — the user cannot drag
 * width that does not exist. `'stacked'` is the only answer that keeps the
 * inspector present without wedging the column, so the 800 px boundary is
 * pinned on both sides.
 */
describe('pickInspectorLayout', () => {
  it('derives its threshold from the pane minimums the divider clamps against', () => {
    expect(INSPECTOR_SIDE_MIN_WIDTH).toBe(DEFAULT_MIN_PRIMARY + DEFAULT_MIN_SECONDARY);
    expect(INSPECTOR_SIDE_MIN_WIDTH).toBe(800);
  });

  it('stacks before anything has been measured', () => {
    expect(pickInspectorLayout(null)).toBe('stacked');
  });

  it('stacks one pixel below the threshold', () => {
    expect(pickInspectorLayout({ width: INSPECTOR_SIDE_MIN_WIDTH - 1, height: 900 })).toBe(
      'stacked',
    );
  });

  it('goes side-by-side exactly at the threshold', () => {
    expect(pickInspectorLayout({ width: INSPECTOR_SIDE_MIN_WIDTH, height: 900 })).toBe('side');
  });

  it('goes side-by-side on a laptop column, well below the meta-track threshold', () => {
    expect(pickInspectorLayout({ width: 1280, height: 800 })).toBe('side');
    expect(pickRunLayout({ width: 1280, height: 800 })).toBe('stacked');
  });

  it('stacks on a zero height, however wide', () => {
    expect(pickInspectorLayout({ width: 3400, height: 0 })).toBe('stacked');
  });

  it('stacks on a negative width', () => {
    expect(pickInspectorLayout({ width: -1600, height: 900 })).toBe('stacked');
  });
});

/**
 * The claim: the opening width is one the divider can hand back.
 *
 * A default outside the drag range is a width the user is stuck with in one
 * direction, so every answer here has to survive the same clamp the divider
 * applies — which is why the degenerate widths are pinned alongside the
 * proportional ones.
 */
describe('defaultInspectorWidth', () => {
  it('opens on a proportion of the column once there is room for one', () => {
    expect(defaultInspectorWidth({ width: 1440, height: 900 })).toBe(480);
    expect(defaultInspectorWidth({ width: 2400, height: 1200 })).toBe(800);
  });

  it('keeps the run surface the larger pane', () => {
    const width = 1920;
    const secondary = defaultInspectorWidth({ width, height: 1080 });
    expect(secondary).toBeLessThan(width - secondary);
  });

  it('clamps up to the pane minimum where a third is not enough', () => {
    const size = { width: INSPECTOR_SIDE_MIN_WIDTH, height: 900 };
    expect(size.width * INSPECTOR_WIDTH_FRACTION).toBeLessThan(DEFAULT_MIN_SECONDARY);
    expect(defaultInspectorWidth(size)).toBe(DEFAULT_MIN_SECONDARY);
  });

  it('answers the pane minimum for a column that was never measured', () => {
    expect(defaultInspectorWidth(null)).toBe(DEFAULT_MIN_SECONDARY);
    expect(defaultInspectorWidth({ width: 0, height: 900 })).toBe(DEFAULT_MIN_SECONDARY);
    expect(defaultInspectorWidth({ width: -800, height: 900 })).toBe(DEFAULT_MIN_SECONDARY);
  });

  it('never starves the primary pane on a column too narrow to hold both', () => {
    const secondary = defaultInspectorWidth({ width: 600, height: 900 });
    expect(secondary).toBeLessThanOrEqual(600 - DEFAULT_MIN_PRIMARY);
  });
});
