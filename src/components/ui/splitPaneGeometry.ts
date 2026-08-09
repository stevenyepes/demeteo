/**
 * Every constraint the `SplitPane` divider obeys, as arithmetic over numbers.
 *
 * The component this serves resizes without rendering (UI_REDESIGN_PLAN §4.1):
 * a drag writes a CSS custom property straight onto the container, so no width
 * it computes mid-drag ever reaches React, and none of it is visible in
 * rendered output. Keeping the rules here is what makes them checkable at all —
 * the alternative is asserting against a jsdom that lays nothing out.
 *
 * The secondary pane's width is the one number the whole model is expressed in,
 * because it is the value a caller persists and the value the divider's
 * `aria-valuenow` reports; the primary pane takes whatever is left.
 */

export interface SplitBounds {
  /** Measured width of the box both panes share, in px. */
  containerWidth: number;
  /** Narrowest primary pane still worth rendering, in px. */
  minPrimary: number;
  /** Narrowest secondary pane still worth rendering, in px. */
  minSecondary: number;
}

export const DEFAULT_MIN_PRIMARY = 480;
export const DEFAULT_MIN_SECONDARY = 320;

/**
 * Share of `minSecondary` below which a drag collapses the pane instead of
 * clamping it.
 *
 * Without a collapse zone the divider simply stops, and a user dragging it to
 * the edge — which is how a user asks for the whole column — gets a pane
 * wedged at its minimum that they then have to close some other way. Half the
 * minimum is far enough past it that no clamped drag lands there by accident.
 */
export const COLLAPSE_FRACTION = 0.5;

/** Pixels one arrow key moves the divider. */
export const KEYBOARD_STEP = 24;

function isMeasured(bounds: SplitBounds): boolean {
  return bounds.containerWidth > 0;
}

/** Widest the secondary pane may be without starving the primary one. */
export function maxSecondaryWidth(bounds: SplitBounds): number {
  return Math.max(0, Math.round(bounds.containerWidth - bounds.minPrimary));
}

/** Requested width at or below which a drag collapses the secondary pane. */
export function collapseBelow(bounds: SplitBounds): number {
  return Math.round(bounds.minSecondary * COLLAPSE_FRACTION);
}

/**
 * Width the secondary pane takes for a width the *drag* asked for — the only
 * entry point that may answer `0`.
 *
 * An unmeasured container (width `0`, which is every mount before layout and
 * every jsdom render) applies no constraint rather than clamping against
 * zeros: the same rule `pickRunLayout` follows, for the same reason — a
 * collapse decision taken from a container that was never laid out is a
 * decision taken from nothing.
 */
export function resolveSecondaryWidth(requested: number, bounds: SplitBounds): number {
  const width = Math.round(requested);
  if (!isMeasured(bounds)) return Math.max(0, width);

  const max = maxSecondaryWidth(bounds);
  if (max <= 0) return 0;
  if (width <= collapseBelow(bounds)) return 0;

  return clampOpen(width, bounds, max);
}

/**
 * Width the secondary pane takes when it is being *opened* — restoring a
 * persisted width, reopening after a collapse, or any keyboard move.
 *
 * Deliberately cannot collapse: a keypress that closed the pane outright
 * because the arithmetic crossed a threshold is indistinguishable from a bug,
 * and the divider offers `Home` and its collapse toggle for closing it on
 * purpose.
 */
export function openSecondaryWidth(requested: number, bounds: SplitBounds): number {
  const width = Math.round(requested);
  if (!isMeasured(bounds)) return Math.max(0, width);

  const max = maxSecondaryWidth(bounds);
  if (max <= 0) return 0;

  return clampOpen(width, bounds, max);
}

function clampOpen(width: number, bounds: SplitBounds, max: number): number {
  const min = Math.min(Math.round(bounds.minSecondary), max);
  return Math.min(Math.max(width, min), max);
}

/** Secondary width for a pointer at `pointerX`, given the container's right edge. */
export function secondaryWidthFromPointer(pointerX: number, containerRight: number): number {
  return containerRight - pointerX;
}

/** Secondary width after one keyboard step of `delta` px. */
export function nudgeSecondaryWidth(current: number, delta: number, bounds: SplitBounds): number {
  const from = Math.round(current);
  const step = Math.round(delta);
  if (from <= 0) return step > 0 ? openSecondaryWidth(bounds.minSecondary, bounds) : 0;
  return openSecondaryWidth(from + step, bounds);
}

/**
 * Secondary width after a collapse toggle. `lastExpanded` is the width to come
 * back to; `0` means there is none yet and the pane opens at its minimum.
 */
export function toggleSecondaryWidth(
  current: number,
  lastExpanded: number,
  bounds: SplitBounds,
): number {
  if (Math.round(current) > 0) return 0;
  return openSecondaryWidth(lastExpanded > 0 ? lastExpanded : bounds.minSecondary, bounds);
}

/**
 * Secondary width a keystroke on the divider asks for, or `null` when the key
 * belongs to the rest of the app.
 *
 * The whole key map is a decision, so it lives here beside the widths it
 * produces rather than as a `switch` inside the component (UI_REDESIGN_PLAN
 * §5.2). Left grows the secondary pane because that is the direction the
 * divider moves, not the direction the pane grows in.
 */
export function secondaryWidthForKey(
  key: string,
  current: number,
  lastExpanded: number,
  bounds: SplitBounds,
): number | null {
  if (!isMeasured(bounds)) return null;

  switch (key) {
    case 'ArrowLeft':
    case 'ArrowUp':
      return nudgeSecondaryWidth(current, KEYBOARD_STEP, bounds);
    case 'ArrowRight':
    case 'ArrowDown':
      return nudgeSecondaryWidth(current, -KEYBOARD_STEP, bounds);
    case 'Home':
      return 0;
    case 'End':
      return openSecondaryWidth(maxSecondaryWidth(bounds), bounds);
    case 'Enter':
    case ' ':
      return toggleSecondaryWidth(current, lastExpanded, bounds);
    default:
      return null;
  }
}
