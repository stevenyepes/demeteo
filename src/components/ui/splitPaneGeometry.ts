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

/** Pixels one arrow key moves the divider. */
export const KEYBOARD_STEP = 24;

function isMeasured(bounds: SplitBounds): boolean {
  return bounds.containerWidth > 0;
}

/** Widest the secondary pane may be without starving the primary one. */
export function maxSecondaryWidth(bounds: SplitBounds): number {
  return Math.max(0, Math.round(bounds.containerWidth - bounds.minPrimary));
}

/**
 * The width the secondary pane takes for any width anything asks for — a drag,
 * a keystroke, or a caller's persisted value. There is one clamp because there
 * is one rule: **the pane clamps to its minimum and never collapses to zero**
 * (UI_REDESIGN_PLAN §7, settled 2026-08-08). An earlier model snapped the pane
 * shut below half its minimum; nothing in this file may reintroduce a width
 * that means "closed", because no caller has a way back from one.
 *
 * An unmeasured container (width `0`, which is every mount before layout and
 * every jsdom render) applies no constraint rather than clamping against zeros:
 * the same rule `pickRunLayout` follows, for the same reason — a width taken
 * from a container that was never laid out is a width taken from nothing.
 *
 * This *can* still answer `0`, and the distinction matters: when the container
 * is too narrow to seat `minPrimary + minSecondary`, the primary minimum wins
 * and the max it leaves may reach zero. That is a measurement degenerate case
 * with no width available to give, not a collapse affordance — the pane
 * reappears the moment the container can hold it, with nothing to reopen.
 */
export function resolveSecondaryWidth(requested: number, bounds: SplitBounds): number {
  const width = Math.round(requested);
  if (!isMeasured(bounds)) return Math.max(0, width);

  const max = maxSecondaryWidth(bounds);
  const min = Math.min(Math.round(bounds.minSecondary), max);
  return Math.min(Math.max(width, min), max);
}

/** Secondary width for a pointer at `pointerX`, given the container's right edge. */
export function secondaryWidthFromPointer(pointerX: number, containerRight: number): number {
  return containerRight - pointerX;
}

/** Secondary width after one keyboard step of `delta` px. */
export function nudgeSecondaryWidth(current: number, delta: number, bounds: SplitBounds): number {
  return resolveSecondaryWidth(Math.round(current) + Math.round(delta), bounds);
}

/**
 * Secondary width a keystroke on the divider asks for, or `null` when the key
 * belongs to the rest of the app.
 *
 * The whole key map is a decision, so it lives here beside the widths it
 * produces rather than as a `switch` inside the component (UI_REDESIGN_PLAN
 * §5.2). Left grows the secondary pane because that is the direction the
 * divider moves, not the direction the pane grows in. `Enter` and `Space` are
 * claimed by nothing: they toggled the pane closed, and there is no closed
 * state left to toggle.
 */
export function secondaryWidthForKey(
  key: string,
  current: number,
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
      return resolveSecondaryWidth(bounds.minSecondary, bounds);
    case 'End':
      return resolveSecondaryWidth(maxSecondaryWidth(bounds), bounds);
    default:
      return null;
  }
}
