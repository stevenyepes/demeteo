/**
 * Which gates the run chrome's persistent strip is announcing
 * (`docs/UI_REDESIGN_PLAN.md` §3.2, §5.2).
 *
 * Membership is the literal `awaiting_gate` step status, deliberately *not*
 * `runStatusMeta().tone === 'amber' && !active` — the rule `pipelineFilter.ts`
 * uses for the project view's "needs you" band. The two look like the same
 * question and are not: that one segments *features*, where amber-and-settled
 * does mean a human is blocked, while this one names *steps*, where
 * `interrupted` is equally amber and settled and has no gate to decide. A tone
 * match here would put a Decide CTA on a step whose `gate_decide` the backend
 * refuses outright.
 */

/** The fields this module reads off a step row. */
export interface GateStripRow {
  id: string;
  step_id: string;
  step_index: number;
  status: string;
}

/**
 * The steps waiting on a human decision, earliest first.
 *
 * A DAG can hold several open gates on independent branches, so this is a list
 * rather than the single `gateStepExecutionId` the navigation state carries;
 * the strip counts them all and acts on the first. Ordering is `step_index`
 * with a position tiebreak, because a replay opens a second execution of a
 * step at the same index and the two must not swap places between renders.
 */
export function awaitingGates<T extends GateStripRow>(steps: readonly T[]): T[] {
  return steps
    .map((step, index) => ({ step, index }))
    .filter((entry) => entry.step.status === 'awaiting_gate')
    .sort((a, b) => a.step.step_index - b.step.step_index || a.index - b.index)
    .map((entry) => entry.step);
}
