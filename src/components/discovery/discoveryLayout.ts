/**
 * How the discovery workspace row should use the width it was actually given.
 *
 * The row is three panes wide at most — Interview, Tickets, and the ticket
 * inspector. It degrades in two steps rather than one: the inspector floats
 * over the graph before the row gives up the third column entirely, and only
 * once Interview and Tickets cannot both hold their minimum does it fall back
 * to showing one pane at a time. The ticket *editor* takes no part in this: it
 * is a modal, sized to the window, so nothing here has to seat it.
 *
 * The interview's width is the user's — a divider they drag — so every
 * threshold below is computed from the width it will actually render at, not
 * from a constant.
 *
 * The verdicts live here rather than in `DiscoveryWorkspaceRow` because they
 * *are* policy decisions — nothing here reads the DOM or measures anything,
 * and each is answerable from a test with plain numbers. The component's job
 * is only to measure the row and pass the numbers in.
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

/** The ticket inspector's fixed width, in flow or floating. */
export const INSPECTOR_WIDTH = 360;

/** The interview's width until the user drags it. */
export const DEFAULT_INTERVIEW_WIDTH = 440;

/** Narrowest interview whose question cards and composer still read. */
export const INTERVIEW_MIN_WIDTH = 340;

/** Past this a transcript line is too long to read, whatever the window. */
export const INTERVIEW_MAX_WIDTH = 820;

/** Pixels one arrow key moves the interview's divider. */
export const INTERVIEW_KEYBOARD_STEP = 24;

/** `InterviewCollapsedRail`'s width — what a hidden interview leaves behind. */
export const COLLAPSED_RAIL_WIDTH = 40;

/**
 * The width the interview renders at for a width the user asked for.
 *
 * The graph's minimum outranks the user's choice: a window narrowed after the
 * divider was dragged would otherwise starve the graph until they dragged
 * again. Nothing is written back for it, so the chosen width returns when the
 * room does. An unmeasured row (`0`) applies only the interview's own bounds —
 * a width taken from a row that was never laid out is a width taken from
 * nothing.
 */
export function resolveInterviewWidth(requested: number, rowWidth: number): number {
  const width = Number.isFinite(requested) ? Math.round(requested) : DEFAULT_INTERVIEW_WIDTH;
  const ceiling =
    rowWidth > 0
      ? Math.min(INTERVIEW_MAX_WIDTH, Math.round(rowWidth) - GRAPH_MIN_WIDTH)
      : INTERVIEW_MAX_WIDTH;
  return Math.max(INTERVIEW_MIN_WIDTH, Math.min(width, ceiling));
}

/**
 * The interview width a keystroke on its divider asks for, or `null` when the
 * key belongs to the rest of the app. The divider sits on the interview's right
 * edge, so Right grows it — the direction the divider moves.
 */
export function interviewWidthForKey(
  key: string,
  current: number,
  rowWidth: number,
): number | null {
  switch (key) {
    case 'ArrowRight':
    case 'ArrowUp':
      return resolveInterviewWidth(current + INTERVIEW_KEYBOARD_STEP, rowWidth);
    case 'ArrowLeft':
    case 'ArrowDown':
      return resolveInterviewWidth(current - INTERVIEW_KEYBOARD_STEP, rowWidth);
    case 'Home':
      return resolveInterviewWidth(INTERVIEW_MIN_WIDTH, rowWidth);
    case 'End':
      return resolveInterviewWidth(INTERVIEW_MAX_WIDTH, rowWidth);
    default:
      return null;
  }
}

/**
 * Pick the discovery row's layout for the space it has.
 *
 * `'three-up'` requires the interview (or its rail), the inspector and the
 * graph's minimum to fit side by side at the widths they would render at.
 * `'overlay-inspector'` requires the interview's *minimum* plus the graph's —
 * the interview narrows to fit before the row gives up a pane. Everything else
 * — nothing measured yet, a zero or negative width, a collapsed height —
 * answers `'stacked'`, the mode that works at every size. A hidden or
 * not-yet-laid-out row reports zeros, and that must not read as "wide".
 *
 * `interviewHidden` is the user's collapse toggle. It is an *input* and never
 * an output — `'stacked'` picks one pane at a time and offers the interview as
 * one of them, so the caller ignores the collapse there, and this function
 * returning `'stacked'` must not feed back into the flag.
 */
export function pickDiscoveryLayout(
  size: DiscoveryRowSize | null,
  interviewHidden = false,
  interviewWidth = DEFAULT_INTERVIEW_WIDTH,
): DiscoveryLayoutMode {
  if (!size) return 'stacked';
  if (size.width <= 0 || size.height <= 0) return 'stacked';
  const leftMin = interviewHidden ? COLLAPSED_RAIL_WIDTH : INTERVIEW_MIN_WIDTH;
  if (size.width < leftMin + GRAPH_MIN_WIDTH) return 'stacked';
  const left = interviewHidden
    ? COLLAPSED_RAIL_WIDTH
    : resolveInterviewWidth(interviewWidth, size.width);
  if (size.width >= left + INSPECTOR_WIDTH + GRAPH_MIN_WIDTH) return 'three-up';
  return 'overlay-inspector';
}
