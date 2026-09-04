/**
 * How the discovery workspace row should use the width it was actually given.
 *
 * The row is three panes wide at most — Interview, Tickets, and the ticket
 * inspector — and today's fixed-width layout only works once all three fit.
 * Below that it degrades in two steps rather than one: the inspector moves to
 * an overlay before the row gives up the third column entirely, and only once
 * neither Interview nor Tickets can hold its own minimum does it fall back to
 * showing one pane at a time.
 *
 * The verdicts live here rather than in `DiscoveryWorkspaceRow` because they
 * *are* policy decisions — nothing here reads the DOM or measures anything,
 * and each is answerable from a test with two numbers. The component's job is
 * only to measure the row and pass the numbers in.
 */

export interface DiscoveryRowSize {
  width: number;
  height: number;
}

export type DiscoveryLayoutMode = 'three-up' | 'overlay-inspector' | 'stacked';

/**
 * Floor for the ticket graph/board pane to stay usable.
 *
 * One column of the graph's 280px nodes plus padding, and no narrower than
 * the inspector it stands beside.
 */
export const GRAPH_MIN_WIDTH = 360;

/**
 * Row width at which Interview and the graph/board can still sit side by
 * side once the inspector stops claiming a column.
 *
 * `560` (interview, fixed) + `GRAPH_MIN_WIDTH` (`360`).
 */
export const OVERLAY_MIN_WIDTH = 920;

/**
 * Row width at which all three columns fit at once, matching today's layout.
 *
 * `560` (interview) + `360` (inspector) + `GRAPH_MIN_WIDTH` (`360`).
 */
export const THREE_UP_MIN_WIDTH = 1280;

/**
 * Pick the discovery row's layout for the space it has.
 *
 * `'three-up'` requires a real measurement at or above `THREE_UP_MIN_WIDTH`.
 * `'overlay-inspector'` requires at least `OVERLAY_MIN_WIDTH`. Everything
 * else — nothing measured yet, a zero or negative width, a collapsed height —
 * answers `'stacked'`, which is the mode that works at every size. A hidden
 * or not-yet-laid-out row reports zeros, and that must not read as "wide".
 */
export function pickDiscoveryLayout(size: DiscoveryRowSize | null): DiscoveryLayoutMode {
  if (!size) return 'stacked';
  if (size.width <= 0 || size.height <= 0) return 'stacked';
  if (size.width >= THREE_UP_MIN_WIDTH) return 'three-up';
  if (size.width >= OVERLAY_MIN_WIDTH) return 'overlay-inspector';
  return 'stacked';
}
