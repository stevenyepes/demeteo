import { describe, expect, it } from 'vitest';

import type { PendingGate, RunEvent, StepView } from './runs';
import { sameStatusMap, statusByNode, ticketRows } from './runView';

function gate(stepId: string | null, stepExecutionId: string): PendingGate {
  return { featureId: 'f1', stepExecutionId, stepId, since: 100 };
}

function event(overrides: Partial<RunEvent>): RunEvent {
  return {
    offset: 0,
    kind: 'step_started',
    occurredAt: 0,
    stepId: null,
    stepExecutionId: null,
    ticketId: null,
    ...overrides,
  };
}

describe('statusByNode', () => {
  it('passes running, completed and failed through', () => {
    const steps: StepView[] = [
      { stepId: 'build', state: 'running' },
      { stepId: 'research', state: 'completed' },
      { stepId: 'test', state: 'failed' },
    ];

    expect(statusByNode(steps, [])).toEqual({
      build: { status: 'running' },
      research: { status: 'completed' },
      test: { status: 'failed' },
    });
  });

  it('marks a pending gate awaiting_gate with its execution id', () => {
    expect(statusByNode([], [gate('approve', 'exec-1')])).toEqual({
      approve: { status: 'awaiting_gate', stepExecutionId: 'exec-1' },
    });
  });

  it('lets a pending gate win over running for the same step', () => {
    const steps: StepView[] = [{ stepId: 'approve', state: 'running' }];

    expect(statusByNode(steps, [gate('approve', 'exec-1')])).toEqual({
      approve: { status: 'awaiting_gate', stepExecutionId: 'exec-1' },
    });
  });

  it('infers nothing for a step the reply does not report', () => {
    const map = statusByNode([{ stepId: 'build', state: 'running' }], [gate(null, 'exec-2')]);

    expect(Object.keys(map)).toEqual(['build']);
    expect(map).not.toHaveProperty('deploy');
  });
});

describe('sameStatusMap', () => {
  it('is true for equal maps built separately', () => {
    const steps: StepView[] = [{ stepId: 'build', state: 'running' }];
    const gates = [gate('approve', 'exec-1')];

    expect(sameStatusMap(statusByNode(steps, gates), statusByNode(steps, gates))).toBe(true);
  });

  it('is false when a status, an execution id or the key set differs', () => {
    const base = statusByNode([{ stepId: 'build', state: 'running' }], [gate('approve', 'exec-1')]);

    expect(sameStatusMap(base, statusByNode([{ stepId: 'build', state: 'completed' }], [gate('approve', 'exec-1')]))).toBe(
      false,
    );
    expect(sameStatusMap(base, statusByNode([{ stepId: 'build', state: 'running' }], [gate('approve', 'exec-2')]))).toBe(
      false,
    );
    expect(sameStatusMap(base, statusByNode([{ stepId: 'build', state: 'running' }], []))).toBe(false);
    expect(sameStatusMap(statusByNode([{ stepId: 'build', state: 'running' }], []), base)).toBe(false);
  });
});

describe('ticketRows', () => {
  it('keeps one row per ticket holding its latest event', () => {
    const events = [
      event({ offset: 1, kind: 'ticket_started', occurredAt: 10, ticketId: 't1' }),
      event({ offset: 2, kind: 'ticket_started', occurredAt: 11, ticketId: 't2' }),
      event({ offset: 3, kind: 'ticket_completed', occurredAt: 20, ticketId: 't1' }),
    ];

    expect(ticketRows(events)).toEqual([
      { ticketId: 't1', lastKind: 'ticket_completed', lastAt: 20 },
      { ticketId: 't2', lastKind: 'ticket_started', lastAt: 11 },
    ]);
  });

  it('orders by occurredAt, then offset, not by position in the reply', () => {
    const events = [
      event({ offset: 9, kind: 'ticket_failed', occurredAt: 30, ticketId: 't1' }),
      event({ offset: 4, kind: 'ticket_started', occurredAt: 10, ticketId: 't1' }),
      event({ offset: 8, kind: 'verdict', occurredAt: 30, ticketId: 't1' }),
    ];

    expect(ticketRows(events)).toEqual([{ ticketId: 't1', lastKind: 'ticket_failed', lastAt: 30 }]);
  });

  it('ignores events that name no ticket', () => {
    const events = [
      event({ offset: 1, kind: 'step_started', occurredAt: 10 }),
      event({ offset: 2, kind: 'ticket_started', occurredAt: 5, ticketId: 't1' }),
      event({ offset: 3, kind: 'step_completed', occurredAt: 40 }),
    ];

    expect(ticketRows(events)).toEqual([{ ticketId: 't1', lastKind: 'ticket_started', lastAt: 5 }]);
  });
});
