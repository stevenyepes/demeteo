/**
 * How much the Discovery workspace header puts inline, decided from the
 * header's own measured width (UI_REDESIGN_PLAN §5.1 — a rule that decides what
 * should happen does not live inside the component that renders it).
 *
 * **Measure the header, not the window.** The header shares the window with
 * the project rail, and a split-screen window is where it runs out of room
 * first: at roughly 1000px the inline action group alone wanted ~950px, and
 * because it could not shrink, the title column collapsed to nothing and the
 * lifecycle chip drew over the metrics. The header spans its column and keeps
 * that width whichever density it renders, so there is no feedback loop to
 * damp and no band is needed.
 *
 * - `wide` — every action inline, plus the ticket-count chip.
 * - `compact` — Decompose stays inline as the primary; Close, Update and the
 *   integration PR move into the actions menu.
 * - `narrow` — `compact`, with the lifecycle chip reduced to a dot and the
 *   stats line dropping tokens.
 *
 * The thresholds are estimates from the class sizes, not WebKitGTK
 * measurements: the wide row is ~1240px of fixed content (four labelled
 * buttons, two chips, crumb, a 120px title floor, `px-5` padding), so 1360
 * leaves the title real room; the compact row is ~600px, so 760 is where the
 * crumb and the chip start costing the title its floor. Re-measure before
 * adding an action to the inline group.
 *
 * `width <= 0` returning `current` is what keeps jsdom — which lays nothing
 * out — and every first frame at the initial density.
 */

export type DiscoveryHeaderDensity = 'wide' | 'compact' | 'narrow';

/** At or above this measured header width, every action renders inline. */
export const DISCOVERY_HEADER_WIDE_AT_PX = 1360;
/** Below this measured header width, the header renders `narrow`. */
export const DISCOVERY_HEADER_NARROW_BELOW_PX = 760;

export function nextDiscoveryHeaderDensity(
  width: number,
  current: DiscoveryHeaderDensity,
): DiscoveryHeaderDensity {
  if (width <= 0) return current;
  if (width >= DISCOVERY_HEADER_WIDE_AT_PX) return 'wide';
  if (width < DISCOVERY_HEADER_NARROW_BELOW_PX) return 'narrow';
  return 'compact';
}
