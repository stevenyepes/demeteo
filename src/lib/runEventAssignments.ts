import { EFFORT_LABELS, isEffortLevel, type EffortLevel } from './effortLevels';

/** What one `agent_spawned` payload says, before it is placed in a log. */
export interface AssignmentEvidence {
  stepExecutionId: string;
  agentKind: string;
  effort: EffortLevel | null;
}

export interface RunEventAssignment extends AssignmentEvidence {
  offset: number;
}

export type RunEventAssignments = Record<string, RunEventAssignment>;

export interface AssignmentRunEvent {
  offset: number;
  kind: string;
  payload_json: unknown;
}

export const NO_INJECTED_EFFORT_LABEL = 'No injected effort';

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

/**
 * What an `agent_spawned` row asserts, with no reference to where in a log it
 * sits — which is what a renderer of a single event needs, and all it needs.
 * Offset is a reconciliation concern (which spawn wins for an execution) and
 * enters only through [`parseRunEventAssignment`].
 */
export function parseAssignmentEvidence(
  kind: string,
  payloadJson: unknown,
): AssignmentEvidence | null {
  if (kind !== 'agent_spawned' || typeof payloadJson !== 'string') return null;

  let payload: unknown;
  try {
    payload = JSON.parse(payloadJson) as unknown;
  } catch {
    return null;
  }

  if (!isRecord(payload)) return null;
  if (!isNonEmptyString(payload.step_execution_id)) return null;
  if (!isNonEmptyString(payload.agent_kind)) return null;
  // An absent `effort` key is rejected along with an unparseable one, so a
  // payload predating the field reads as *no spawn evidence* rather than as
  // one of the three states this model does carry. The alternative is to show
  // it as `null`, which asserts "no effort was injected" — a claim the payload
  // does not make. There is no fourth "unknown" rung, and inventing one on the
  // read side would put it in the UI only, where it could never be produced by
  // a spawn that actually happened.
  if (payload.effort !== null && !isEffortLevel(payload.effort)) return null;

  return {
    stepExecutionId: payload.step_execution_id,
    agentKind: payload.agent_kind,
    effort: payload.effort,
  };
}

/** [`parseAssignmentEvidence`] stamped with the durable offset it arrived at. */
export function parseRunEventAssignment(event: unknown): RunEventAssignment | null {
  if (!isRecord(event)) return null;
  if (typeof event.offset !== 'number' || !Number.isFinite(event.offset)) return null;

  const evidence = parseAssignmentEvidence(String(event.kind), event.payload_json);
  return evidence && { ...evidence, offset: event.offset };
}

export function reconcileRunEventAssignments(
  current: RunEventAssignments,
  events: readonly unknown[],
): RunEventAssignments {
  let next = current;

  for (const event of events) {
    const assignment = parseRunEventAssignment(event);
    if (!assignment) continue;

    const existing = next[assignment.stepExecutionId];
    if (existing && existing.offset >= assignment.offset) continue;

    next = { ...next, [assignment.stepExecutionId]: assignment };
  }

  return next;
}

export function assignmentEffortLabel(effort: EffortLevel | null): string {
  return effort === null ? NO_INJECTED_EFFORT_LABEL : EFFORT_LABELS[effort];
}

/**
 * The one accessible name for an assignment badge pair, so the canvas and the
 * timeline announce the same fact with the same words. `subject` names what is
 * being annotated (a node title, a step name) — the badges sit inside cards
 * that are visually self-locating and are not, to a screen reader.
 */
export function assignmentAriaLabel(
  subject: string,
  agentKind: string,
  effortLabel: string,
): string {
  return `Actual assignment for ${subject}: Agent: ${agentKind}; Effective effort: ${effortLabel}`;
}
