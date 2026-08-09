/**
 * How one `step_executions` row is dressed as the thing the inspector inspects.
 *
 * The inspector body is `NodePanel`, which was written for the canvas and asks
 * for a graph node plus the overlay entry the canvas paints. Serving the
 * timeline from the same panel means answering both for a run that may have no
 * graph at all — a legacy feature, or a definition that failed to load — so the
 * fallbacks live here, pure and testable, rather than as `??` chains inside the
 * component.
 */

import type { NodeConfigV2, NodeRunStatus, WorkflowDefinitionV2 } from '../canvas/types';
import type { StepExecution } from '../../types';

/** `s-write-tests` → `Write Tests`. The step id is the only name a run without
 *  a graph definition has for its steps. */
export function humanizeStepId(id: string): string {
  return id
    .replace(/^s-/, '')
    .split('-')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/**
 * The graph node backing a step, synthesized when the run has no definition.
 *
 * `step_kind` is the synthetic node's `type` because the two vocabularies are
 * the same one — `nodeTypeMeta` falls back to a neutral glyph for anything it
 * does not know, so an unrecognised kind degrades to a plain row rather than
 * mislabelling itself as an agent.
 */
export function inspectorNodeConfig(
  graphDef: WorkflowDefinitionV2 | null,
  step: StepExecution,
): NodeConfigV2 {
  const node = graphDef?.nodes.find((n) => n.id === step.step_id);
  if (node) return node;
  return { id: step.step_id, type: step.step_kind, title: humanizeStepId(step.step_id) };
}

/**
 * Overlay state for the *selected execution*, not for its node.
 *
 * `useRunEvents` keys `statusByNode` by node id and keeps the most recently
 * updated execution, which is the right answer for painting a canvas node and
 * the wrong one here: a selection may name an older attempt, and reading the
 * node's entry would then show one execution's status over another's attempt
 * history. The failure class is the exception — it arrives on the
 * `retry_decision` run-event, which the step row does not carry, so it is the
 * one field taken from the node's entry.
 */
export function inspectorRunStatus(
  step: StepExecution,
  errorClass: string | null | undefined,
): NodeRunStatus {
  return {
    status: step.status,
    costUsd: step.cost_usd ?? null,
    wallClockSecs: step.wall_clock_secs ?? null,
    tokens: step.tokens ?? null,
    errorClass: errorClass ?? null,
    stepExecutionId: step.id,
  };
}
