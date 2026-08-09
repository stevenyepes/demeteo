/**
 * What the step inspector shows, decided outside the component that renders it
 * (UI redesign plan §3.1, §5.2).
 *
 * The pipeline view has one inspector served by both run surfaces, and the
 * selection that drives it lives in navigation state — so it survives a
 * back/forward, a deep link, and a reload that replaced every step row. That
 * makes "the selected id resolves to nothing" a routine state rather than an
 * error, and resolving it is a policy decision: an id that no longer exists
 * must degrade to a named empty state, never throw and never quietly resolve to
 * a neighbouring step the user did not pick and might act on.
 */

import { findActivePredecessor, type GateBlocker } from './features';
import type { StepExecution } from '../types';

/**
 * Why the inspector has nothing to show. Split three ways because the panel
 * words them differently, and an empty panel that cannot say *why* it is empty
 * is the thing that reads as broken: "pick a step" is an invitation, "that step
 * is gone" explains a stale link, and "the run has no steps yet" is progress.
 */
export type InspectorEmptyReason = 'no-steps' | 'no-selection' | 'stale-selection';

export type InspectorTarget =
  | { kind: 'empty'; reason: InspectorEmptyReason }
  | { kind: 'step'; step: StepExecution; blockedBy: GateBlocker | null };

/**
 * Statuses whose inspector action the backend refuses while an ancestor is
 * still live — `step_retry` (`domain/run_control.rs::retry_refusal`) and
 * `gate_decide`. Only these get a `blockedBy`, because that field exists to
 * explain a *disabled action*: a `pending` step is waiting on its predecessor
 * by design, so naming one there is noise, and `replay_from_step` carries no
 * such guard at all.
 */
const BLOCKABLE_STATUSES = ['failed', 'interrupted', 'awaiting_gate'];

/**
 * Resolve `selectedStepId` against the run's steps.
 *
 * The id may be either a `step_executions` id (what the timeline selects) or a
 * graph node id (what the canvas selects); one selection key has to serve both
 * views. An execution id wins, and a node id resolves to its most recently
 * updated execution, so a retried node shows the attempt in progress rather
 * than the one that failed first.
 *
 * An empty run answers `no-steps` even with a selection in hand: the reason the
 * panel is empty is that nothing has been planned yet, which is not the same
 * story as a step that vanished.
 */
export function inspectorTarget(
  steps: readonly StepExecution[],
  selectedStepId: string | null,
): InspectorTarget {
  if (steps.length === 0) return { kind: 'empty', reason: 'no-steps' };
  if (!selectedStepId) return { kind: 'empty', reason: 'no-selection' };

  const step = resolveSelection(steps, selectedStepId);
  if (!step) return { kind: 'empty', reason: 'stale-selection' };

  return { kind: 'step', step, blockedBy: resolveBlocker(steps, step) };
}

/**
 * The step to select in a run the user has not touched: the one that needs
 * them, else the one in motion, else the last one that finished. Returns an
 * execution id `inspectorTarget` resolves, or `null` only for an empty run.
 *
 * A waiting gate outranks an earlier failure because it is the question the run
 * is blocked on, and a DAG can hold both at once on independent branches.
 * Within a tier the *earliest* step wins — the first thing that went wrong
 * explains the ones after it — except for a finished run, where the last step
 * to settle is the outcome.
 */
export function defaultInspectorSelection(steps: readonly StepExecution[]): string | null {
  const pick =
    earliest(steps, (s) => s.status === 'awaiting_gate') ??
    earliest(steps, (s) => s.status === 'failed' || s.status === 'interrupted') ??
    earliest(steps, (s) => s.status === 'running' || s.status === 'verifying') ??
    latest(steps, (s) => s.status === 'completed' || s.status === 'skipped') ??
    earliest(steps, () => true);
  return pick?.id ?? null;
}

function resolveSelection(
  steps: readonly StepExecution[],
  selectedStepId: string,
): StepExecution | null {
  const exact = steps.find((s) => s.id === selectedStepId);
  if (exact) return exact;

  let newest: StepExecution | null = null;
  for (const s of steps) {
    if (s.step_id !== selectedStepId) continue;
    if (!newest || s.updated_at >= newest.updated_at) newest = s;
  }
  return newest;
}

function resolveBlocker(
  steps: readonly StepExecution[],
  step: StepExecution,
): GateBlocker | null {
  if (!BLOCKABLE_STATUSES.includes(step.status)) return null;
  const pred = findActivePredecessor(steps, step);
  if (!pred) return null;
  return {
    id: pred.id,
    step_id: pred.step_id,
    status: pred.status,
    step_index: pred.step_index,
  };
}

type StepPredicate = (step: StepExecution) => boolean;

function earliest(steps: readonly StepExecution[], match: StepPredicate): StepExecution | null {
  let found: StepExecution | null = null;
  for (const s of steps) {
    if (!match(s)) continue;
    if (!found || s.step_index < found.step_index) found = s;
  }
  return found;
}

function latest(steps: readonly StepExecution[], match: StepPredicate): StepExecution | null {
  let found: StepExecution | null = null;
  for (const s of steps) {
    if (!match(s)) continue;
    if (!found || s.step_index > found.step_index) found = s;
  }
  return found;
}
