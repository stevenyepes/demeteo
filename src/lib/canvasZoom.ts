/**
 * The zoom arithmetic the two hand-rolled canvases share — `TicketGraph` and
 * `AskCanvasView`, neither of which is React Flow (`docs/TASKS_DISCOVERY.md`
 * records why the ticket graph is not). Both had their own copy of a clamp and
 * a fit; this is the one copy, and it is pure so the decisions are reachable
 * from a test without a DOM.
 *
 * The fit factor is the load-bearing part. A fit computed against the pane's
 * full box lands the content exactly on the pane, and *exactly* is the one
 * value that oscillates: content at the pane's size makes the scrollbars
 * appear, a scrollbar shrinks the pane's content box, the `ResizeObserver`
 * fires, and the refit is a different number than the one that armed it. The
 * margin means a fitted canvas never overflows, so a fit is a fixed point.
 */

export const ZOOM_MIN = 0.4;
export const ZOOM_MAX = 2;

/** One press of the +/− buttons. Multiplicative rather than additive: a fixed
 *  0.15 step is a third of the graph at 0.45 and a fourteenth at 2.0, so the
 *  same click does visibly different work at each end of the range. */
const STEP_RATIO = 1.2;

/** Wheel travel → zoom factor, as `e^(-pixels · RATE)`. Exponential for the
 *  same reason the button step is multiplicative, and so that a notch and its
 *  opposite cancel exactly. One 100px notch ≈ 16%. */
const WHEEL_RATE = 0.0015;

const LINE_PX = 16;
const PAGE_PX = 400;

/** Fraction of the pane a fit is allowed to fill — see the module note. */
const FIT_MARGIN = 0.92;

export interface Size {
  width: number;
  height: number;
}

export function clampZoom(zoom: number): number {
  return round(Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom)));
}

export function steppedZoom(zoom: number, direction: 1 | -1): number {
  return clampZoom(zoom * STEP_RATIO ** direction);
}

/** `deltaY` is only in pixels when `deltaMode` says so — Firefox reports lines
 *  and a page-scrolling mouse reports pages, and either read as a pixel count
 *  is a zoom that barely moves. */
export function wheelPixels(deltaY: number, deltaMode: number): number {
  if (deltaMode === 1) return deltaY * LINE_PX;
  if (deltaMode === 2) return deltaY * PAGE_PX;
  return deltaY;
}

export function wheelZoom(zoom: number, pixels: number): number {
  return clampZoom(zoom * Math.exp(-pixels * WHEEL_RATE));
}

/** Framing shrinks to fit and never magnifies: a two-ticket plan in a wide
 *  pane would otherwise open at 2× — cards the size of a dialog, which is not
 *  what anyone asked "fit" for. Zooming past 1:1 stays something the operator
 *  does on purpose. */
export function fitZoom(pane: Size, content: Size): number {
  if (content.width <= 0 || content.height <= 0) return 1;
  return clampZoom(
    Math.min(
      1,
      (pane.width * FIT_MARGIN) / content.width,
      (pane.height * FIT_MARGIN) / content.height,
    ),
  );
}

/**
 * Where the scroller has to land for the point under the cursor to stay under
 * the cursor across a zoom — the difference between a zoom that reads as
 * magnification and one that reads as the diagram running away.
 *
 * `offset` is the cursor's distance from the scaled canvas's own edge, so it
 * already carries whatever centring margin the canvas has; dividing by the
 * outgoing zoom recovers the unscaled coordinate under the cursor, and the
 * scroller moves by what that coordinate gains. Clamping is the browser's —
 * assigning past `scrollWidth` lands at the end.
 */
export function anchoredScroll(
  scroll: number,
  offset: number,
  zoom: number,
  next: number,
): number {
  return scroll + (offset / zoom) * (next - zoom);
}

export function zoomPercent(zoom: number): string {
  return `${Math.round(zoom * 100)}%`;
}

function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}
