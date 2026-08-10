/**
 * Which step `j` and `k` move to, decided outside the keydown handler that
 * fires them (AGENTS.md §3, UI_REDESIGN_PLAN §3.6, §5.2).
 *
 * A sibling of `inspectorTarget.ts` rather than an addition to it, and it takes
 * its anchor by *calling* that module: a move is relative to the step the
 * inspector is showing, and the rules that turn one selection key into that
 * step — an execution id, a graph node id, a retried node's latest attempt —
 * are already spelled once there. Copied here they would be a second answer to
 * "which step is selected", free to disagree with the one on screen.
 *
 * The view mode does not enter into it: both surfaces are fed the same `steps`
 * array and a DAG offers no linear order of its own, so "next" means the row
 * below on either, and the keys carry over.
 */

import { inspectorTarget } from './inspectorTarget';
import type { StepExecution } from '../types';

export type StepDirection = 'next' | 'previous';

/**
 * The execution id to select, or `null` when the selection does not move.
 *
 * **Clamped, not wrapped.** Holding `j` down a 30-step run stops at the last
 * step instead of reappearing at the first, where the inspector's contents
 * would have jumped the length of the run with nothing on screen to account for
 * it. `ui/rovingIndex.ts` wraps because a three-item strip is entirely visible
 * and the move is a single tab; a run column is neither.
 *
 * An unresolved selection — cleared, or naming a step the run no longer has —
 * sits *outside* the list rather than at an end of it, so `j` enters at the
 * first step and `k` at the last: refusing to move would leave the keys dead in
 * the one state where they are the cheapest way back.
 *
 * Array order, not `step_index` arithmetic: the timeline renders `steps` as the
 * backend hands them over, so a re-sort here would let `j` skip a visible row,
 * and index gaps left by a replay would stall it.
 */
export function adjacentStepSelection(
  steps: readonly StepExecution[],
  selectedStepId: string | null,
  direction: StepDirection,
): string | null {
  if (steps.length === 0) return null;

  const shown = inspectorTarget(steps, selectedStepId);
  if (shown.kind !== 'step') {
    return direction === 'next' ? steps[0].id : steps[steps.length - 1].id;
  }

  const to = steps.indexOf(shown.step) + (direction === 'next' ? 1 : -1);
  return to >= 0 && to < steps.length ? steps[to].id : null;
}
