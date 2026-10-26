import type { NodeRunStatus } from '../../../src/components/canvas/types';
import type { PendingGate, RunEvent, StepView } from './runs';

export type TicketRow = { ticketId: string; lastKind: string; lastAt: number };

/**
 * Only what the mirror reported: a step with no event and a gate naming no
 * step are absent, never drawn as `pending`. A gate overrides its step's own
 * state, which reads `running` while the instance waits on the decision.
 */
export function statusByNode(steps: StepView[], pendingGates: PendingGate[]): Record<string, NodeRunStatus> {
  const map: Record<string, NodeRunStatus> = {};
  for (const step of steps) map[step.stepId] = { status: step.state };
  for (const gate of pendingGates) {
    if (gate.stepId !== null) map[gate.stepId] = { status: 'awaiting_gate', stepExecutionId: gate.stepExecutionId };
  }
  return map;
}

/** By value over the fields {@link statusByNode} sets; the canvas re-seeds whenever the map's identity changes. */
export function sameStatusMap(a: Record<string, NodeRunStatus>, b: Record<string, NodeRunStatus>): boolean {
  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  return keys.every((key) => {
    const other = b[key];
    return (
      other !== undefined &&
      other.status === a[key].status &&
      (other.stepExecutionId ?? null) === (a[key].stepExecutionId ?? null)
    );
  });
}

export function ticketRows(events: RunEvent[]): TicketRow[] {
  const latest = new Map<string, RunEvent>();
  for (const event of events) {
    if (event.ticketId === null) continue;
    const held = latest.get(event.ticketId);
    if (held === undefined || isLater(event, held)) latest.set(event.ticketId, event);
  }
  return [...latest.entries()].map(([ticketId, event]) => ({ ticketId, lastKind: event.kind, lastAt: event.occurredAt }));
}

function isLater(event: RunEvent, than: RunEvent): boolean {
  return event.occurredAt !== than.occurredAt ? event.occurredAt > than.occurredAt : event.offset > than.offset;
}
