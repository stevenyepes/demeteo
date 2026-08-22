import { EFFORT_LABELS, isEffortLevel, type EffortLevel } from './effortLevels';

export interface RunEventAssignment {
  stepExecutionId: string;
  agentKind: string;
  effort: EffortLevel | null;
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

export function parseRunEventAssignment(event: unknown): RunEventAssignment | null {
  if (!isRecord(event) || event.kind !== 'agent_spawned') return null;
  if (typeof event.offset !== 'number' || !Number.isFinite(event.offset)) return null;
  if (typeof event.payload_json !== 'string') return null;

  let payload: unknown;
  try {
    payload = JSON.parse(event.payload_json) as unknown;
  } catch {
    return null;
  }

  if (!isRecord(payload)) return null;
  if (!isNonEmptyString(payload.step_execution_id)) return null;
  if (!isNonEmptyString(payload.agent_kind)) return null;
  if (payload.effort !== null && !isEffortLevel(payload.effort)) return null;

  return {
    stepExecutionId: payload.step_execution_id,
    agentKind: payload.agent_kind,
    effort: payload.effort,
    offset: event.offset,
  };
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
