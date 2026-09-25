/**
 * Whether a step in `status` may be re-pointed at a different harness, model
 * or effort. Mirrors `domain::step_assignment::assignment_refusal`
 * (`crates/demeteo-core/src/domain/step_assignment.rs`), which is the
 * authority: only the two statuses that mean a process is already up are
 * refused, because the spawn for that attempt read its assignment before
 * either was reached. Everything else — including a status this build has
 * never heard of — is assignable, which is what makes the rule predictable to
 * a user: the pin applies whenever that node next runs.
 *
 * The Rust↔TS mirroring is unavoidable and nothing mechanical can see it
 * drift, so this is the one place to revisit when the Rust rule changes — a
 * surface that re-derives the split instead of calling this would keep
 * offering a control the backend refuses, and `checks.sh` would stay green.
 *
 * It answers only the assignability question. Liveness (`runStatusMeta().active`,
 * the pulsing node) and stream affordances test the same two strings today for
 * unrelated reasons; routing those through here would tie them to a rule that
 * can gain a third refused status without them.
 */
export function isAssignable(status: string): boolean {
  return status !== 'running' && status !== 'verifying';
}

const ASSIGNABLE_KINDS: readonly string[] = ['agent', 'sequence', 'parallel', 'sync', 'finalize'];

/**
 * Whether a node of `kind` has an assignment at all. Mirrors
 * `domain::step_assignment::ASSIGNABLE_KINDS`, under the same drift warning as
 * `isAssignable` above: a gate or command node spawns no agent, so a pin on it
 * would be accepted, drawn as planned, and never read.
 */
export function takesAssignment(kind: string): boolean {
  return ASSIGNABLE_KINDS.includes(kind);
}

/**
 * Whether a node's stored pin reads as *planned* on its card. A node that
 * finished without spawn evidence (`agent_spawned` not captured) would
 * otherwise show its pin as a plan for a run that is already over; the pin
 * still applies to a replay, and the inspector still shows it.
 */
export function showsPlanned(status: string | undefined): boolean {
  return status !== 'completed' && status !== 'skipped';
}
