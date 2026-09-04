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

/** The ticket inspector's fixed width, when it holds a column of its own. */
export const INSPECTOR_WIDTH = 360;

/** The interview column's fixed width. */
export const INTERVIEW_WIDTH = 560;

/** `InterviewCollapsedRail`'s width — what a hidden interview leaves behind. */
export const COLLAPSED_RAIL_WIDTH = 40;

/**
 * What hiding the interview actually hands back to the row.
 *
 * Not `INTERVIEW_WIDTH`: the column is replaced by a rail, not by nothing, and
 * that rail is the only control that brings the interview back — so it is
 * never absent from a row the collapse applies to. Reclaiming the full column
 * overdraws every threshold below by `COLLAPSED_RAIL_WIDTH` and squeezes the
 * graph under `GRAPH_MIN_WIDTH`, which `TicketColumn` accepts silently: it is
 * `flex-1 min-w-0`, so it shrinks rather than overflowing and nothing reports
 * the pane is now too narrow to read.
 */
export const RECLAIMED_BY_HIDING = INTERVIEW_WIDTH - COLLAPSED_RAIL_WIDTH;

/**
 * Row width at which Interview and the graph/board can still sit side by
 * side once the inspector stops claiming a column.
 *
 * Summed rather than spelled: a threshold written as a literal beside a
 * comment claiming what it adds up to is a comment that can go stale on its
 * own, and this one is the arithmetic the whole file is about.
 */
export const OVERLAY_MIN_WIDTH = INTERVIEW_WIDTH + GRAPH_MIN_WIDTH;

/** Row width at which all three columns fit at once. */
export const THREE_UP_MIN_WIDTH = INTERVIEW_WIDTH + INSPECTOR_WIDTH + GRAPH_MIN_WIDTH;

/**
 * Pick the discovery row's layout for the space it has.
 *
 * `'three-up'` requires a real measurement at or above `THREE_UP_MIN_WIDTH`.
 * `'overlay-inspector'` requires at least `OVERLAY_MIN_WIDTH`. Everything
 * else — nothing measured yet, a zero or negative width, a collapsed height —
 * answers `'stacked'`, which is the mode that works at every size. A hidden
 * or not-yet-laid-out row reports zeros, and that must not read as "wide".
 *
 * `interviewHidden` is the user's collapse toggle, and it moves the ladder
 * rather than sitting beside it: the room the interview was holding is real
 * room — `RECLAIMED_BY_HIDING` of it — so a row too narrow to have shown the
 * inspector in flow can show it once the interview is out of the way, and
 * every pane still clears the minimum it was given. It is an *input* and
 * never an output — `'stacked'` picks one pane at a time and offers the
 * interview as one of them, so the caller ignores the collapse there, and this
 * function returning `'stacked'` must not feed back into the flag.
 */
export function pickDiscoveryLayout(
  size: DiscoveryRowSize | null,
  interviewHidden = false,
): DiscoveryLayoutMode {
  if (!size) return 'stacked';
  if (size.width <= 0 || size.height <= 0) return 'stacked';
  const reclaimed = interviewHidden ? RECLAIMED_BY_HIDING : 0;
  if (size.width >= THREE_UP_MIN_WIDTH - reclaimed) return 'three-up';
  if (size.width >= OVERLAY_MIN_WIDTH - reclaimed) return 'overlay-inspector';
  return 'stacked';
}
