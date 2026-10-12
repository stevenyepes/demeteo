import { describe, expect, it } from 'vitest';

import {
  awaitingGates,
  gateHeldReason,
  type GateHeldEvent,
  type GateStripRow,
} from './gateStrip';

function step(id: string, status: string, step_index: number): GateStripRow {
  return { id, step_id: `s-${id}`, step_index, status };
}

function ids(rows: readonly GateStripRow[]): string[] {
  return rows.map((r) => r.id);
}

describe('awaitingGates', () => {
  it('returns nothing while no step is waiting on a decision', () => {
    expect(awaitingGates([step('a', 'running', 0), step('b', 'completed', 1)])).toEqual([]);
    expect(awaitingGates([])).toEqual([]);
  });

  it('keeps only the steps waiting on a decision', () => {
    const steps = [
      step('a', 'completed', 0),
      step('b', 'awaiting_gate', 1),
      step('c', 'running', 2),
      step('d', 'awaiting_gate', 3),
    ];

    expect(ids(awaitingGates(steps))).toEqual(['b', 'd']);
  });

  it('orders by step_index, not by the order the run rows arrived in', () => {
    const steps = [
      step('late', 'awaiting_gate', 7),
      step('early', 'awaiting_gate', 2),
      step('mid', 'awaiting_gate', 4),
    ];

    expect(ids(awaitingGates(steps))).toEqual(['early', 'mid', 'late']);
  });

  it('keeps two executions of one index in the order they arrived', () => {
    const steps = [step('first', 'awaiting_gate', 3), step('replay', 'awaiting_gate', 3)];

    expect(ids(awaitingGates(steps))).toEqual(['first', 'replay']);
  });

  it('leaves the caller its list untouched', () => {
    const steps = [step('b', 'awaiting_gate', 5), step('a', 'awaiting_gate', 1)];

    awaitingGates(steps);

    expect(ids(steps)).toEqual(['b', 'a']);
  });

  // Every other amber-and-settled status: each one would join the strip if
  // membership were derived from the tone the way `pipelineFilter` derives its
  // "needs you" band, and none of them has a gate to decide.
  it.each(['interrupted', 'needs-credentials', 'bootstrapping', 'parked', 'gated', 'failed'])(
    'leaves a %s step out of the strip',
    (status) => {
      expect(awaitingGates([step('a', status, 0)])).toEqual([]);
    },
  );
});

function held(step_execution_id: string, reason: unknown): GateHeldEvent {
  return { kind: 'gate_held_by_review', payload_json: JSON.stringify({ step_execution_id, reason }) };
}

describe('gateHeldReason', () => {
  it('gives the reason the engine recorded for that gate', () => {
    expect(gateHeldReason([held('se-ship', 'the review lists critical or major issues')], 'se-ship')).toBe(
      'the review lists critical or major issues',
    );
  });

  it('gives nothing for a gate no policy would have approved', () => {
    const events = [{ kind: 'gate_required', payload_json: JSON.stringify({ step_execution_id: 'se-ship' }) }];

    expect(gateHeldReason(events, 'se-ship')).toBeNull();
    expect(gateHeldReason([], 'se-ship')).toBeNull();
  });

  it('does not borrow the reason another gate was held for', () => {
    expect(gateHeldReason([held('se-other', 'no counts')], 'se-ship')).toBeNull();
  });

  // A rework cycle reaches the same gate again, with a new review.
  it('gives the latest reason when the gate was held more than once', () => {
    const events = [held('se-ship', 'the review carries no issue counts'), held('se-ship', 'now blocking')];

    expect(gateHeldReason(events, 'se-ship')).toBe('now blocking');
  });

  it('skips an event whose payload it cannot read rather than failing the strip', () => {
    const events: GateHeldEvent[] = [
      held('se-ship', 'readable'),
      { kind: 'gate_held_by_review', payload_json: '{not json' },
      { kind: 'gate_held_by_review', payload_json: null },
      { kind: 'gate_held_by_review', payload_json: '"a string"' },
      held('se-ship', 42),
      held('se-ship', ''),
    ];

    expect(gateHeldReason(events, 'se-ship')).toBe('readable');
  });
});
