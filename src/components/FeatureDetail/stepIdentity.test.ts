import { describe, expect, it } from 'vitest';

import type { WorkflowDefinitionV2 } from '../canvas/types';
import type { StepExecution } from '../../types';
import { humanizeStepId, inspectorNodeConfig, inspectorRunStatus } from './stepIdentity';

const step = (over: Partial<StepExecution> = {}): StepExecution => ({
  id: 'se-1',
  feature_id: 'f-1',
  step_id: 's-write-tests',
  step_index: 2,
  step_kind: 'agent',
  status: 'completed',
  artifact_paths: [],
  created_at: 0,
  updated_at: 0,
  ...over,
});

const graph = (): WorkflowDefinitionV2 => ({
  schema_version: 2,
  id: 'w-1',
  name: 'Standard',
  nodes: [{ id: 's-write-tests', type: 'sequence', title: 'Write the tests' }],
  edges: [],
});

describe('humanizeStepId', () => {
  it('reads a step id as a title', () => {
    expect(humanizeStepId('s-write-tests')).toBe('Write Tests');
    expect(humanizeStepId('finalize')).toBe('Finalize');
  });
});

describe('inspectorNodeConfig', () => {
  it('prefers the stored node, which carries the author’s title and type', () => {
    expect(inspectorNodeConfig(graph(), step())).toEqual(graph().nodes[0]);
  });

  it('synthesizes one for a run with no definition', () => {
    expect(inspectorNodeConfig(null, step())).toEqual({
      id: 's-write-tests',
      type: 'agent',
      title: 'Write Tests',
    });
  });

  it('synthesizes one for a step the definition does not contain', () => {
    // A replay can outlive an edit to the workflow, and a pinned definition can
    // be missing a step the run has rows for. Either way the panel still opens.
    const node = inspectorNodeConfig(graph(), step({ step_id: 's-hotfix', step_kind: 'command' }));
    expect(node).toEqual({ id: 's-hotfix', type: 'command', title: 'Hotfix' });
  });
});

describe('inspectorRunStatus', () => {
  it('describes the selected execution, not its node', () => {
    const older = step({ id: 'se-old', status: 'failed', cost_usd: 0.25, wall_clock_secs: 12, tokens: 900 });
    expect(inspectorRunStatus(older, null)).toEqual({
      status: 'failed',
      costUsd: 0.25,
      wallClockSecs: 12,
      tokens: 900,
      errorClass: null,
      stepExecutionId: 'se-old',
    });
  });

  it('takes the failure class from the overlay, the one field a step row lacks', () => {
    expect(inspectorRunStatus(step({ status: 'failed' }), 'environment').errorClass).toBe(
      'environment',
    );
  });

  it("carries the selected execution's launch evidence", () => {
    const status = inspectorRunStatus(step({ id: 'se-2', status: 'running' }), null, {
      stepExecutionId: 'se-2',
      agentKind: 'claude-code',
      model: 'sonnet',
      effort: 'max',
    });
    expect(status.agentKind).toBe('claude-code');
    expect(status.model).toBe('sonnet');
    expect(status.effort).toBe('max');
  });

  it('leaves the assignment fields absent where no spawn was recorded', () => {
    const bare = inspectorRunStatus(step({ status: 'running' }), null);
    expect('agentKind' in bare).toBe(false);
    expect('model' in bare).toBe(false);
    expect('effort' in bare).toBe(false);
  });

  it('reads absent metrics as null rather than undefined', () => {
    // `NodeRunStatus` is rendered straight into the Overview tab, where
    // `undefined` and `null` are the same on screen but not to a strict test —
    // and the canvas's own overlay normalizes the same way.
    const bare = inspectorRunStatus(step(), undefined);
    expect(bare.costUsd).toBeNull();
    expect(bare.wallClockSecs).toBeNull();
    expect(bare.tokens).toBeNull();
    expect(bare.errorClass).toBeNull();
  });
});
