/**
 * The claim: every width the splitter can produce is a width both panes can
 * live with, and no width it produces closes the secondary pane — it clamps to
 * the minimum instead (UI_REDESIGN_PLAN §7).
 *
 * These are pinned here rather than by dragging a real window because the
 * splitter's whole reason for existing (UI_REDESIGN_PLAN §4.1) is that the drag
 * path touches no React state — so the drag path is not observable through
 * rendered output, and the constraints have to be answerable without a DOM.
 */
import { describe, expect, it } from 'vitest';

import {
  DEFAULT_MIN_PRIMARY,
  DEFAULT_MIN_SECONDARY,
  KEYBOARD_STEP,
  maxSecondaryWidth,
  nudgeSecondaryWidth,
  resolveSecondaryWidth,
  secondaryWidthForKey,
  secondaryWidthFromPointer,
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

  it('keeps the secondary minimum itself', () => {
    expect(resolveSecondaryWidth(320, BOUNDS)).toBe(320);
  });

  it('holds the minimum however far past it the request goes', () => {
    expect(resolveSecondaryWidth(200, BOUNDS)).toBe(320);
    expect(resolveSecondaryWidth(161, BOUNDS)).toBe(320);
    expect(resolveSecondaryWidth(160, BOUNDS)).toBe(320);
    expect(resolveSecondaryWidth(12, BOUNDS)).toBe(320);
    expect(resolveSecondaryWidth(0, BOUNDS)).toBe(320);
  });

  it('holds the minimum for a request dragged past the container edge', () => {
    expect(resolveSecondaryWidth(-200, BOUNDS)).toBe(320);
  });

  it('gives the secondary pane what is left when both minima cannot fit', () => {
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: 700 })).toBe(220);
    expect(resolveSecondaryWidth(100, { ...BOUNDS, containerWidth: 700 })).toBe(220);
  });

  it('has nothing to give when the container cannot hold the primary minimum', () => {
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: 400 })).toBe(0);
  });

  it('applies no constraint at all before the container is measured', () => {
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: 0 })).toBe(400);
    expect(resolveSecondaryWidth(400, { ...BOUNDS, containerWidth: -1 })).toBe(400);
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

  it('floors at the secondary minimum', () => {
    expect(nudgeSecondaryWidth(330, -KEYBOARD_STEP, BOUNDS)).toBe(320);
    expect(nudgeSecondaryWidth(320, -KEYBOARD_STEP, BOUNDS)).toBe(320);
  });

  it('caps at the width the primary minimum leaves', () => {
    expect(nudgeSecondaryWidth(715, KEYBOARD_STEP, BOUNDS)).toBe(720);
  });
});

describe('secondaryWidthForKey', () => {
  it('moves the divider left to grow the secondary pane', () => {
    expect(secondaryWidthForKey('ArrowLeft', 400, BOUNDS)).toBe(424);
    expect(secondaryWidthForKey('ArrowUp', 400, BOUNDS)).toBe(424);
  });

  it('moves the divider right to shrink it', () => {
    expect(secondaryWidthForKey('ArrowRight', 400, BOUNDS)).toBe(376);
    expect(secondaryWidthForKey('ArrowDown', 400, BOUNDS)).toBe(376);
  });

  it('jumps to the narrowest and the widest secondary pane', () => {
    expect(secondaryWidthForKey('Home', 400, BOUNDS)).toBe(320);
    expect(secondaryWidthForKey('End', 400, BOUNDS)).toBe(720);
  });

  it('claims no key that used to close the pane', () => {
    expect(secondaryWidthForKey('Enter', 400, BOUNDS)).toBeNull();
    expect(secondaryWidthForKey(' ', 400, BOUNDS)).toBeNull();
  });

  it('claims no other key', () => {
    expect(secondaryWidthForKey('Tab', 400, BOUNDS)).toBeNull();
    expect(secondaryWidthForKey('a', 400, BOUNDS)).toBeNull();
  });

  it('claims nothing at all before the container is measured', () => {
    const unmeasured = { ...BOUNDS, containerWidth: 0 };
    expect(secondaryWidthForKey('ArrowLeft', 400, unmeasured)).toBeNull();
    expect(secondaryWidthForKey('End', 400, unmeasured)).toBeNull();
  });
});

describe('defaults', () => {
  it('pins the pane minima and the keyboard step', () => {
    expect(DEFAULT_MIN_PRIMARY).toBe(480);
    expect(DEFAULT_MIN_SECONDARY).toBe(320);
    expect(KEYBOARD_STEP).toBe(24);
  });
});
