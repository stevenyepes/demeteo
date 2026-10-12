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

/** The fields this module reads off a run event. */
export interface GateHeldEvent {
  kind: string;
  payload_json: string | null;
}

/** Kinds that record an answer to a gate, a person's or the policy's. */
const GATE_ANSWERED = new Set(['gate_decided', 'gate_auto_approved']);

/**
 * Why the project's own gate autonomy did not approve this gate, when that is
 * why it is waiting — the engine's `gate_held_by_review` run event.
 *
 * Only a reason nothing has answered since. A gate that is redirected and
 * reached again keeps its execution id, and the second visit may be held for
 * no reason at all — the project was switched to asking at every gate in
 * between — so the first visit's reason ends at the answer it was given.
 * `null` for a gate nothing would have approved: most gates wait because the
 * project asks at every gate, and there is no reason to give for that.
 */
export function gateHeldReason(
  events: readonly GateHeldEvent[],
  stepExecutionId: string,
): string | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const event = events[i];
    const held = event.kind === 'gate_held_by_review';
    if ((!held && !GATE_ANSWERED.has(event.kind)) || !event.payload_json) continue;
    let payload: unknown;
    try {
      payload = JSON.parse(event.payload_json);
    } catch {
      continue;
    }
    if (typeof payload !== 'object' || payload === null) continue;
    const { step_execution_id, reason } = payload as Record<string, unknown>;
    if (step_execution_id !== stepExecutionId) continue;
    if (!held) return null;
    if (typeof reason === 'string' && reason) return reason;
  }
  return null;
}
