/**
 * The claim: every width the splitter can produce is a width both panes can
 * live with, and a drag that runs the secondary pane out of room collapses it
 * instead of leaving a sliver.
 *
 * These are pinned here rather than by dragging a real window because the
 * splitter's whole reason for existing (UI_REDESIGN_PLAN §4.1) is that the drag
 * path touches no React state — so the drag path is not observable through
 * rendered output, and the constraints have to be answerable without a DOM.
 */
import { describe, expect, it } from 'vitest';

import {
  COLLAPSE_FRACTION,
  DEFAULT_MIN_PRIMARY,
  DEFAULT_MIN_SECONDARY,
  KEYBOARD_STEP,
  collapseBelow,
  maxSecondaryWidth,
  nudgeSecondaryWidth,
  openSecondaryWidth,
  resolveSecondaryWidth,
  secondaryWidthForKey,
  secondaryWidthFromPointer,
  toggleSecondaryWidth,
} from './splitPaneGeometry';

/** 1200px container, so the secondary pane may have 320..720. */
const BOUNDS = { containerWidth: 1200, minPrimary: 480, minSecondary: 320 };

describe('maxSecondaryWidth', () => {
  it('leaves the primary pane its minimum', () => {
    expect(maxSecondaryWidth(BOUNDS)).toBe(720);
  });

  it('is zero when the container cannot even hold the primary minimum', () => {
    expect(maxSecondaryWidth({ ...BOUNDS, containerWidth: 300 })).toBe(0);
  });
});

describe('collapseBelow', () => {
  it('sits at a fraction of the secondary minimum', () => {
    expect(collapseBelow(BOUNDS)).toBe(160);
  });
});

describe('resolveSecondaryWidth', () => {
  it('passes a comfortable width through', () => {
    expect(resolveSecondaryWidth(500, BOUNDS)).toBe(500);
  });

  it('rounds to whole pixels', () => {
    expect(resolveSecondaryWidth(500.4, BOUNDS)).toBe(500);
  });

  it('clamps to the width the primary minimum leaves', () => {
    expect(resolveSecondaryWidth(900, BOUNDS)).toBe(720);
  });

  it('snaps up to the secondary minimum inside the clamp band', () => {
    expect(resolveSecondaryWidth(200, BOUNDS)).toBe(320);
  });

  it('keeps the secondary minimum itself', () => {
    expect(resolveSecondaryWidth(320, BOUNDS)).toBe(320);
  });

  it('collapses one pixel below the minimum band', () => {
    expect(resolveSecondaryWidth(161, BOUNDS)).toBe(320);
    expect(resolveSecondaryWidth(160, BOUNDS)).toBe(0);
  });

  it('collapses rather than returning a sliver', () => {
    expect(resolveSecondaryWidth(12, BOUNDS)).toBe(0);
  });

  it('collapses a negative request', () => {
    expect(resolveSecondaryWidth(-200, BOUNDS)).toBe(0);
  });

  it('collapses when the container cannot hold the primary minimum', () => {
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: 400 })).toBe(0);
  });

  it('gives the secondary pane what is left when both minima cannot fit', () => {
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: 700 })).toBe(220);
  });

  it('applies no constraint at all before the container is measured', () => {
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: 0 })).toBe(400);
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: -1 })).toBe(400);
  });
});

describe('openSecondaryWidth', () => {
  it('never answers collapsed for a measured container', () => {
    expect(openSecondaryWidth(0, BOUNDS)).toBe(320);
    expect(openSecondaryWidth(-500, BOUNDS)).toBe(320);
  });

  it('still honours the primary minimum', () => {
    expect(openSecondaryWidth(5000, BOUNDS)).toBe(720);
  });
});

describe('secondaryWidthFromPointer', () => {
  it('measures back from the right edge of the container', () => {
    expect(secondaryWidthFromPointer(800, 1200)).toBe(400);
  });

  it('goes negative past the right edge, for the caller to resolve', () => {
    expect(secondaryWidthFromPointer(1300, 1200)).toBe(-100);
  });
});

describe('nudgeSecondaryWidth', () => {
  it('grows and shrinks by the delta', () => {
    expect(nudgeSecondaryWidth(400, KEYBOARD_STEP, BOUNDS)).toBe(424);
    expect(nudgeSecondaryWidth(400, -KEYBOARD_STEP, BOUNDS)).toBe(376);
  });

  it('floors at the secondary minimum instead of collapsing', () => {
    expect(nudgeSecondaryWidth(330, -KEYBOARD_STEP, BOUNDS)).toBe(320);
    expect(nudgeSecondaryWidth(320, -KEYBOARD_STEP, BOUNDS)).toBe(320);
  });

  it('reopens a collapsed pane at its minimum', () => {
    expect(nudgeSecondaryWidth(0, KEYBOARD_STEP, BOUNDS)).toBe(320);
  });

  it('stays collapsed when shrunk further', () => {
    expect(nudgeSecondaryWidth(0, -KEYBOARD_STEP, BOUNDS)).toBe(0);
  });

  it('caps at the width the primary minimum leaves', () => {
    expect(nudgeSecondaryWidth(715, KEYBOARD_STEP, BOUNDS)).toBe(720);
  });
});

describe('toggleSecondaryWidth', () => {
  it('collapses an open pane', () => {
    expect(toggleSecondaryWidth(500, 500, BOUNDS)).toBe(0);
  });

  it('restores the last open width', () => {
    expect(toggleSecondaryWidth(0, 620, BOUNDS)).toBe(620);
  });

  it('opens at the minimum when there is no width to restore', () => {
    expect(toggleSecondaryWidth(0, 0, BOUNDS)).toBe(320);
  });

  it('clamps a restored width that no longer fits', () => {
    expect(toggleSecondaryWidth(0, 900, BOUNDS)).toBe(720);
  });
});

describe('secondaryWidthForKey', () => {
  it('moves the divider left to grow the secondary pane', () => {
    expect(secondaryWidthForKey('ArrowLeft', 400, 400, BOUNDS)).toBe(424);
    expect(secondaryWidthForKey('ArrowUp', 400, 400, BOUNDS)).toBe(424);
  });

  it('moves the divider right to shrink it', () => {
    expect(secondaryWidthForKey('ArrowRight', 400, 400, BOUNDS)).toBe(376);
    expect(secondaryWidthForKey('ArrowDown', 400, 400, BOUNDS)).toBe(376);
  });

  it('jumps to collapsed and to the widest secondary pane', () => {
    expect(secondaryWidthForKey('Home', 400, 400, BOUNDS)).toBe(0);
    expect(secondaryWidthForKey('End', 400, 400, BOUNDS)).toBe(720);
  });

  it('toggles on Enter and Space, restoring the last open width', () => {
    expect(secondaryWidthForKey('Enter', 0, 560, BOUNDS)).toBe(560);
    expect(secondaryWidthForKey(' ', 560, 560, BOUNDS)).toBe(0);
  });

  it('claims no other key', () => {
    expect(secondaryWidthForKey('Tab', 400, 400, BOUNDS)).toBeNull();
    expect(secondaryWidthForKey('a', 400, 400, BOUNDS)).toBeNull();
  });

  it('claims nothing at all before the container is measured', () => {
    const unmeasured = { ...BOUNDS, containerWidth: 0 };
    expect(secondaryWidthForKey('ArrowLeft', 400, 400, unmeasured)).toBeNull();
    expect(secondaryWidthForKey('End', 400, 400, unmeasured)).toBeNull();
  });
});

describe('defaults', () => {
  it('pins the pane minima, the collapse fraction and the keyboard step', () => {
    expect(DEFAULT_MIN_PRIMARY).toBe(480);
    expect(DEFAULT_MIN_SECONDARY).toBe(320);
    expect(COLLAPSE_FRACTION).toBe(0.5);
    expect(KEYBOARD_STEP).toBe(24);
  });
});
