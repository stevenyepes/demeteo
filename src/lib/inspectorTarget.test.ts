/**
 * The claim: a selection that no longer resolves degrades to a *named* empty
 * state, and never to a different step.
 *
 * Both failure directions are user-visible. Throwing on a stale id turns a
 * bookmarked deep link into a blank view once the run is reloaded, and quietly
 * substituting a neighbouring step is worse — a user acting on the inspector
 * (retry, decide) would act on something they did not select. The reasons are
 * pinned separately because the panel renders them differently; collapsing
 * them is what makes an empty inspector read as broken.
 */
import { describe, expect, it } from 'vitest';

import { defaultInspectorSelection, inspectorTarget } from './inspectorTarget';
import type { StepExecution } from '../types';

function step(over: Partial<StepExecution> & Pick<StepExecution, 'id'>): StepExecution {
  return {
    feature_id: 'f-1',
    step_id: over.id,
    step_index: 0,
    step_kind: 'agent',
    status: 'pending',
    artifact_paths: [],
    created_at: 1,
    updated_at: 1,
    ...over,
  };
}

describe('inspectorTarget', () => {
  it('is empty with a reason of its own when nothing is selected', () => {
    const steps = [step({ id: 'e-1', status: 'completed' })];
    expect(inspectorTarget(steps, null)).toEqual({ kind: 'empty', reason: 'no-selection' });
  });

  it('is empty when the run has no steps yet', () => {
    expect(inspectorTarget([], null)).toEqual({ kind: 'empty', reason: 'no-steps' });
  });

  it('reports no-steps, not a stale selection, when a deep link outlives the whole run', () => {
    expect(inspectorTarget([], 'e-9')).toEqual({ kind: 'empty', reason: 'no-steps' });
  });

  it('degrades a selection that no longer exists instead of throwing', () => {
    const steps = [step({ id: 'e-1' }), step({ id: 'e-2', step_index: 1 })];
    expect(inspectorTarget(steps, 'e-gone')).toEqual({
      kind: 'empty',
      reason: 'stale-selection',
    });
  });

  it('returns the selected row itself, not a copy', () => {
    const wanted = step({ id: 'e-2', step_index: 1, status: 'completed' });
    const target = inspectorTarget([step({ id: 'e-1' }), wanted], 'e-2');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.step).toBe(wanted);
  });

  it('resolves a graph node id to that node latest execution', () => {
    const first = step({
      id: 'e-1',
      step_id: 'implement',
      status: 'failed',
      updated_at: 100,
    });
    const retried = step({
      id: 'e-2',
      step_id: 'implement',
      status: 'running',
      updated_at: 300,
    });
    const target = inspectorTarget([first, retried], 'implement');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.step).toBe(retried);
  });

  it('prefers an execution id over a node id that spells the same string', () => {
    const decoy = step({ id: 'other', step_id: 'implement', updated_at: 900 });
    const exact = step({ id: 'implement', step_id: 'implement', step_index: 1, updated_at: 1 });
    const target = inspectorTarget([decoy, exact], 'implement');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.step).toBe(exact);
  });

  it('names the blocking predecessor of a failed step', () => {
    const steps = [
      step({ id: 'e-1', step_id: 'plan', step_index: 0, status: 'completed' }),
      step({ id: 'e-2', step_id: 'review', step_index: 1, status: 'running' }),
      step({ id: 'e-3', step_id: 'implement', step_index: 2, status: 'failed' }),
    ];
    const target = inspectorTarget(steps, 'e-3');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy).toEqual({
      id: 'e-2',
      step_id: 'review',
      status: 'running',
      step_index: 1,
    });
  });

  it('leaves a failed step unblocked when every predecessor is terminal', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'completed' }),
      step({ id: 'e-2', step_index: 1, status: 'skipped' }),
      step({ id: 'e-3', step_index: 2, status: 'failed' }),
    ];
    const target = inspectorTarget(steps, 'e-3');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy).toBeNull();
  });

  it('names the blocking predecessor of an interrupted step', () => {
    const steps = [
      step({ id: 'e-1', step_id: 'plan', step_index: 0, status: 'pending' }),
      step({ id: 'e-2', step_index: 1, status: 'interrupted' }),
    ];
    const target = inspectorTarget(steps, 'e-2');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy?.id).toBe('e-1');
  });

  it('names the blocking predecessor of a gate awaiting a decision', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'verifying' }),
      step({ id: 'e-2', step_index: 1, step_kind: 'gate', status: 'awaiting_gate' }),
    ];
    const target = inspectorTarget(steps, 'e-2');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy?.id).toBe('e-1');
  });

  it('does not blame a predecessor for a step that is itself in motion', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'pending' }),
      step({ id: 'e-2', step_index: 1, status: 'running' }),
    ];
    const target = inspectorTarget(steps, 'e-2');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy).toBeNull();
  });

  it('does not blame a predecessor for a step that has not started', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'running' }),
      step({ id: 'e-2', step_index: 1, status: 'pending' }),
    ];
    const target = inspectorTarget(steps, 'e-2');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy).toBeNull();
  });

  it('does not blame a predecessor for a completed step', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'pending' }),
      step({ id: 'e-2', step_index: 1, status: 'completed' }),
    ];
    const target = inspectorTarget(steps, 'e-2');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy).toBeNull();
  });

  it('shows a status it has never heard of rather than an empty panel', () => {
    const steps = [step({ id: 'e-1', status: 'quarantined-by-a-future-release' })];
    const target = inspectorTarget(steps, 'e-1');
    expect(target.kind).toBe('step');
    if (target.kind !== 'step') return;
    expect(target.blockedBy).toBeNull();
  });
});

describe('defaultInspectorSelection', () => {
  it('has nothing to select in a run with no steps', () => {
    expect(defaultInspectorSelection([])).toBeNull();
  });

  it('picks the gate over an earlier failure', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'failed' }),
      step({ id: 'e-2', step_index: 1, status: 'awaiting_gate' }),
      step({ id: 'e-3', step_index: 2, status: 'pending' }),
    ];
    expect(defaultInspectorSelection(steps)).toBe('e-2');
  });

  it('picks the earliest stopped step when no gate is waiting', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'completed' }),
      step({ id: 'e-2', step_index: 1, status: 'interrupted' }),
      step({ id: 'e-3', step_index: 2, status: 'failed' }),
    ];
    expect(defaultInspectorSelection(steps)).toBe('e-2');
  });

  it('picks the step in motion when nothing needs a human', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'completed' }),
      step({ id: 'e-2', step_index: 1, status: 'verifying' }),
      step({ id: 'e-3', step_index: 2, status: 'pending' }),
    ];
    expect(defaultInspectorSelection(steps)).toBe('e-2');
  });

  it('picks the last step that settled once the run is over', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'completed' }),
      step({ id: 'e-2', step_index: 1, status: 'skipped' }),
      step({ id: 'e-3', step_index: 2, status: 'completed' }),
    ];
    expect(defaultInspectorSelection(steps)).toBe('e-3');
  });

  it('picks the first step of a run that has not started', () => {
    const steps = [
      step({ id: 'e-2', step_index: 1, status: 'pending' }),
      step({ id: 'e-1', step_index: 0, status: 'pending' }),
    ];
    expect(defaultInspectorSelection(steps)).toBe('e-1');
  });

  it('reads step_index, not array order', () => {
    const steps = [
      step({ id: 'e-3', step_index: 2, status: 'failed' }),
      step({ id: 'e-2', step_index: 1, status: 'failed' }),
      step({ id: 'e-1', step_index: 0, status: 'completed' }),
    ];
    expect(defaultInspectorSelection(steps)).toBe('e-2');
  });

  it('resolves to a selection the inspector can actually render', () => {
    const steps = [
      step({ id: 'e-1', step_index: 0, status: 'completed' }),
      step({ id: 'e-2', step_index: 1, status: 'running' }),
    ];
    expect(inspectorTarget(steps, defaultInspectorSelection(steps)).kind).toBe('step');
  });
});
