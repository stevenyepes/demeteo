import { describe, expect, it } from 'vitest';

import { awaitingGates, type GateStripRow } from './gateStrip';

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
