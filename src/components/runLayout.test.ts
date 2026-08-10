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
  META_TRACK_FRACTION,
  META_TRACK_MIN_WIDTH,
  metaTrackWidth,
  runPairSize,
  TRACK_GAP,
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
  it('opens on a proportion of the row once there is room for one', () => {
    expect(defaultInspectorWidth({ width: 1440, height: 900 })).toBe(720);
    expect(defaultInspectorWidth({ width: 2400, height: 1200 })).toBe(1200);
  });

  it('never leaves the run surface the narrower pane', () => {
    // Parity is the ceiling, not the target: the run surface is the subject, so
    // an inspector wider than it inverts the view. Everything below the split
    // threshold is the clamp's answer, where the primary keeps its minimum.
    for (const width of [900, 1200, 1920, 2600, 3600]) {
      const secondary = defaultInspectorWidth({ width, height: 1080 });
      expect(secondary).toBeLessThanOrEqual(width - secondary);
    }
  });

  it('lets the clamp decide at the floor, where the fraction cannot', () => {
    // At the narrowest row that seats both panes, the fraction wants more than
    // the primary can spare — so the answer comes from the ceiling
    // (`row - minPrimary`) and every fraction returns the same thing here.
    const size = { width: INSPECTOR_SIDE_MIN_WIDTH, height: 900 };
    expect(size.width * INSPECTOR_WIDTH_FRACTION).toBeGreaterThan(
      size.width - DEFAULT_MIN_PRIMARY,
    );
    expect(defaultInspectorWidth(size)).toBe(DEFAULT_MIN_SECONDARY);
    expect(defaultInspectorWidth(size)).toBe(size.width - DEFAULT_MIN_PRIMARY);
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

describe('metaTrackWidth', () => {
  it('states no width for a track that is not one', () => {
    // Stacked, the meta panels are a full-width block above the graph. A number
    // here would be applied as an inline width and override that.
    expect(metaTrackWidth({ width: 1200, height: 900 }, 'stacked')).toBeNull();
    expect(metaTrackWidth(null, 'split')).toBeNull();
    expect(metaTrackWidth({ width: 0, height: 900 }, 'split')).toBeNull();
  });

  it('takes a share of the column rather than a fixed width', () => {
    const narrow = metaTrackWidth({ width: 1600, height: 900 }, 'split');
    const wide = metaTrackWidth({ width: 3600, height: 900 }, 'split');
    expect(narrow).toBe(Math.round(1600 * META_TRACK_FRACTION));
    // The defect a fixed width had: every pixel a wider window added went to
    // the run surface, which is a fit-to-view canvas and answers extra width
    // with extra empty space. There is deliberately no ceiling to re-create it.
    expect(wide).toBe(Math.round(3600 * META_TRACK_FRACTION));
    expect(wide).toBeGreaterThan(narrow as number);
  });

  it('holds a floor the gate blocks can be read at', () => {
    // Only reachable below SPLIT_MIN_WIDTH today, so the floor is what keeps
    // the share honest if that threshold ever drops.
    expect(metaTrackWidth({ width: 800, height: 900 }, 'split')).toBe(META_TRACK_MIN_WIDTH);
  });
});

describe('runPairSize', () => {
  it('hands back the whole column when there is no track beside it', () => {
    const size = { width: 1200, height: 900 };
    expect(runPairSize(size, 'stacked')).toEqual(size);
    expect(runPairSize(null, 'split')).toBeNull();
  });

  it('spends the meta track and the gap before the inspector is asked anything', () => {
    const size = { width: 3600, height: 900 };
    const meta = metaTrackWidth(size, 'split') as number;
    expect(runPairSize(size, 'split')).toEqual({ width: 3600 - meta - TRACK_GAP, height: 900 });
  });

  it('keeps the divider inside a row it can actually honour', () => {
    // The clamp `defaultInspectorWidth` resolves against has to be the row the
    // SplitPane really gets: resolved against the column it can return a width
    // leaving the primary pane below its own minimum, which the divider then
    // refuses to reproduce — an opening width the user can never drag back to.
    const size = { width: SPLIT_MIN_WIDTH, height: 900 };
    const pair = runPairSize(size, 'split');
    expect(pair!.width).toBeLessThan(size.width);
    expect(defaultInspectorWidth(pair)).toBeLessThanOrEqual(pair!.width - DEFAULT_MIN_PRIMARY);
  });

  it('never reports a negative row for a column narrower than its own track', () => {
    expect(runPairSize({ width: 100, height: 900 }, 'split')!.width).toBe(0);
  });
});
