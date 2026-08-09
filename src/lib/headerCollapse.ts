/**
 * When the feature header collapses as the run column scrolls
 * (UI_REDESIGN_PLAN §5.2 — a rule that decides what should happen does not
 * live inside the component that renders it).
 *
 * The header is a sibling *above* the scrolling run column rather than a child
 * of it, so it can never observe its own scroll: the offset arrives from
 * whatever watches the column, and this answers what to do with it.
 *
 * **Two thresholds, and the second one is not polish.** Collapsing shortens the
 * header, which lengthens the column's viewport, which lets the browser clamp
 * `scrollTop` back down — so under a single threshold the offset that crosses it
 * is also the thing that undoes the crossing, and the header flaps open and shut
 * for as long as the scroll rests there. That is a feedback loop, not a
 * cosmetic flicker. The band between the two is therefore sized wider than the
 * height the collapse removes (the id line, half the vertical padding, one title
 * size step), which is what stops the loop from ever completing a lap. Widening
 * the difference between the header's two states without widening the band
 * re-opens it.
 */

export const HEADER_COLLAPSE_AT_PX = 96;
export const HEADER_EXPAND_BELOW_PX = 24;

export function nextHeaderCollapsed(scrollTop: number, collapsed: boolean): boolean {
  if (scrollTop >= HEADER_COLLAPSE_AT_PX) return true;
  if (scrollTop <= HEADER_EXPAND_BELOW_PX) return false;
  return collapsed;
}
