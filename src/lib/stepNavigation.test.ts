/**
 * The claim: `j` and `k` answer for every state a live run reaches, and never
 * by throwing or by moving somewhere the user cannot see.
 *
 * Two of the states here are not hypothetical. Steps reconcile while the view
 * is open (`stepReconcile.ts`), so the selection can stop resolving under a
 * held key; and the inspector's dismiss control leaves a run with a selection
 * explicitly cleared. Both have to answer *something* — the keys are the
 * cheapest way back into a run, and dead in exactly those states is the same as
 * not shipping them.
 */
import { describe, expect, it } from 'vitest';

import { adjacentStepSelection } from './stepNavigation';
import type { StepExecution } from '../types';

function step(over: Partial<StepExecution> & Pick<StepExecution, 'id'>): StepExecution {
  return {
    feature_id: 'f-1',
    step_id: over.id,
    step_index: 0,
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: [],
    created_at: 1,
    updated_at: 1,
    ...over,
  };
}

const STEPS: StepExecution[] = [
  step({ id: 'e-1', step_id: 's-research', step_index: 0 }),
  step({ id: 'e-2', step_id: 's-implement', step_index: 1 }),
  step({ id: 'e-3', step_id: 's-review', step_index: 2 }),
];

describe('adjacentStepSelection', () => {
  it('moves down and up the run', () => {
    expect(adjacentStepSelection(STEPS, 'e-2', 'next')).toBe('e-3');
    expect(adjacentStepSelection(STEPS, 'e-2', 'previous')).toBe('e-1');
  });

  it('clamps at both ends rather than wrapping', () => {
    // A wrap would move the inspector the length of the run on one keypress,
    // with nothing on screen to account for it.
    expect(adjacentStepSelection(STEPS, 'e-3', 'next')).toBeNull();
    expect(adjacentStepSelection(STEPS, 'e-1', 'previous')).toBeNull();
  });

  it('enters at the near end when nothing is selected', () => {
    expect(adjacentStepSelection(STEPS, null, 'next')).toBe('e-1');
    expect(adjacentStepSelection(STEPS, null, 'previous')).toBe('e-3');
  });

  it('enters at the near end when the selection no longer resolves', () => {
    // The anchor is gone, so there is nothing to be adjacent *to* — and the
    // alternative, refusing to move, strands the keys in the one state a live
    // reconcile can drop the reader into without them touching anything.
    expect(adjacentStepSelection(STEPS, 'e-gone', 'next')).toBe('e-1');
    expect(adjacentStepSelection(STEPS, 'e-gone', 'previous')).toBe('e-3');
  });

  it('has nowhere to go in a run with no steps', () => {
    expect(adjacentStepSelection([], null, 'next')).toBeNull();
    expect(adjacentStepSelection([], 'e-1', 'previous')).toBeNull();
  });

  it('anchors a canvas selection on the step the inspector resolved it to', () => {
    // The graph selects by node id and the timeline by execution id; one key
    // serves both, so `j` from a node id has to land on the row after that
    // node's execution rather than treating the id as absent.
    expect(adjacentStepSelection(STEPS, 's-implement', 'next')).toBe('e-3');
    expect(adjacentStepSelection(STEPS, 's-implement', 'previous')).toBe('e-1');
  });

  it('anchors a retried node on the attempt the inspector is showing', () => {
    const retried: StepExecution[] = [
      step({ id: 'e-1', step_id: 's-research', step_index: 0 }),
      step({ id: 'e-2a', step_id: 's-implement', step_index: 1, updated_at: 5 }),
      step({ id: 'e-2b', step_id: 's-implement', step_index: 1, updated_at: 9 }),
      step({ id: 'e-3', step_id: 's-review', step_index: 2 }),
    ];
    expect(adjacentStepSelection(retried, 's-implement', 'next')).toBe('e-3');
  });

  it('reads array order, not step_index', () => {
    // A replay leaves gaps, and the timeline renders whatever order the list
    // arrives in — a move computed from the index would skip a visible row.
    const sparse: StepExecution[] = [
      step({ id: 'e-1', step_index: 0 }),
      step({ id: 'e-7', step_index: 7 }),
      step({ id: 'e-9', step_index: 9 }),
    ];
    expect(adjacentStepSelection(sparse, 'e-1', 'next')).toBe('e-7');
    expect(adjacentStepSelection(sparse, 'e-9', 'previous')).toBe('e-7');
  });
});
