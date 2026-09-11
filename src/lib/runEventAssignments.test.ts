import { describe, expect, it } from 'vitest';

import { EFFORT_LABELS, EFFORT_LEVELS } from './effortLevels';
import {
  assignmentAriaLabel,
  assignmentEffortLabel,
  assignmentModelLabel,
  HARNESS_DEFAULT_MODEL_LABEL,
  NO_INJECTED_EFFORT_LABEL,
  parseRunEventAssignment,
  reconcileRunEventAssignments,
  type AssignmentRunEvent,
  type RunEventAssignments,
} from './runEventAssignments';

function spawned(
  offset: number,
  payload: unknown,
  overrides: Partial<AssignmentRunEvent> = {},
): AssignmentRunEvent {
  return {
    offset,
    kind: 'agent_spawned',
    payload_json: JSON.stringify(payload),
    ...overrides,
  };
}

describe('parseRunEventAssignment', () => {
  it('parses an ordinary agent spawn and ignores extra fields', () => {
    expect(parseRunEventAssignment(spawned(12, {
      step_execution_id: 'execution-1',
      agent_kind: 'codex',
      effort: 'high',
      model: 'gpt-5.6-codex',
      feature_id: 'feature-1',
      from_a_newer_runner: true,
    }))).toEqual({
      stepExecutionId: 'execution-1',
      agentKind: 'codex',
      effort: 'high',
      model: 'gpt-5.6-codex',
      offset: 12,
    });
  });

  it('reads a pinned model alongside agent and effort', () => {
    expect(parseRunEventAssignment(spawned(5, {
      step_execution_id: 'execution-1',
      agent_kind: 'claude-code',
      effort: 'medium',
      model: 'claude-sonnet-4-5',
    }))).toEqual({
      stepExecutionId: 'execution-1',
      agentKind: 'claude-code',
      effort: 'medium',
      model: 'claude-sonnet-4-5',
      offset: 5,
    });
  });

  it('preserves an explicitly null model', () => {
    const assignment = parseRunEventAssignment(spawned(6, {
      step_execution_id: 'execution-1',
      agent_kind: 'hermes',
      effort: 'low',
      model: null,
    }));

    expect(assignment?.model).toBeNull();
    expect(assignment).toMatchObject({ agentKind: 'hermes', effort: 'low' });
  });

  it('a payload with no `model` key still yields agent + effort', () => {
    const assignment = parseRunEventAssignment(spawned(9, {
      step_execution_id: 'execution-1',
      agent_kind: 'codex',
      effort: 'xhigh',
    }));

    expect(assignment).toEqual({
      stepExecutionId: 'execution-1',
      agentKind: 'codex',
      effort: 'xhigh',
      offset: 9,
    });
    expect(assignment).not.toHaveProperty('model');
  });

  it.each([
    ['a number', 42],
    ['an object', {}],
    ['an empty string', ''],
    ['a whitespace-only string', '   '],
  ])('treats a model that is %s as no model evidence without dropping the spawn', (_case, model) => {
    const assignment = parseRunEventAssignment(spawned(4, {
      step_execution_id: 'execution-1',
      agent_kind: 'opencode',
      effort: null,
      model,
    }));

    expect(assignment).toEqual({
      stepExecutionId: 'execution-1',
      agentKind: 'opencode',
      effort: null,
      offset: 4,
    });
    expect(assignment).not.toHaveProperty('model');
  });

  it('still rejects an invalid effort when the model is valid', () => {
    expect(parseRunEventAssignment(spawned(1, {
      step_execution_id: 'execution-1',
      agent_kind: 'codex',
      effort: 'ultra',
      model: 'gpt-5.6-codex',
    }))).toBeNull();
  });

  it.each(EFFORT_LEVELS)('accepts the supported %s effort', (effort) => {
    expect(parseRunEventAssignment(spawned(1, {
      step_execution_id: 'execution-1',
      agent_kind: 'claude-code',
      effort,
    }))?.effort).toBe(effort);
  });

  it('preserves an explicitly null effort', () => {
    expect(parseRunEventAssignment(spawned(8, {
      step_execution_id: 'execution-1',
      agent_kind: 'hermes',
      model: null,
      effort: null,
    }))).toMatchObject({ agentKind: 'hermes', effort: null });
  });

  it.each([
    ['malformed JSON', { offset: 1, kind: 'agent_spawned', payload_json: '{' }],
    ['null JSON', { offset: 1, kind: 'agent_spawned', payload_json: 'null' }],
    ['array JSON', { offset: 1, kind: 'agent_spawned', payload_json: '[]' }],
    ['non-string payload', { offset: 1, kind: 'agent_spawned', payload_json: { effort: 'high' } }],
    ['missing execution id', spawned(1, { agent_kind: 'codex', effort: 'high' })],
    ['empty execution id', spawned(1, { step_execution_id: '', agent_kind: 'codex', effort: 'high' })],
    ['null execution id', spawned(1, { step_execution_id: null, agent_kind: 'codex', effort: 'high' })],
    ['non-string execution id', spawned(1, { step_execution_id: 42, agent_kind: 'codex', effort: 'high' })],
    ['missing agent kind', spawned(1, { step_execution_id: 'execution-1', effort: 'high' })],
    ['empty agent kind', spawned(1, { step_execution_id: 'execution-1', agent_kind: '', effort: 'high' })],
    ['non-string agent kind', spawned(1, { step_execution_id: 'execution-1', agent_kind: 42, effort: 'high' })],
    ['missing effort', spawned(1, { step_execution_id: 'execution-1', agent_kind: 'codex' })],
    ['unknown effort', spawned(1, { step_execution_id: 'execution-1', agent_kind: 'codex', effort: 'ultra' })],
    ['wrong event kind', spawned(1, { step_execution_id: 'execution-1', agent_kind: 'codex', effort: 'high' }, { kind: 'step_progress' })],
  ])('rejects %s', (_case, event) => {
    expect(parseRunEventAssignment(event)).toBeNull();
  });
});

describe('reconcileRunEventAssignments', () => {
  it('keeps the greatest offset regardless of arrival order or retries', () => {
    const events = [
      spawned(20, { step_execution_id: 'execution-1', agent_kind: 'codex', effort: 'xhigh' }),
      spawned(10, { step_execution_id: 'execution-1', agent_kind: 'opencode', effort: 'low' }),
      spawned(30, { step_execution_id: 'execution-1', agent_kind: 'claude-code', effort: 'medium' }),
      spawned(25, { step_execution_id: 'execution-1', agent_kind: 'hermes', effort: null }),
    ];

    expect(reconcileRunEventAssignments({}, events)).toEqual({
      'execution-1': {
        stepExecutionId: 'execution-1',
        agentKind: 'claude-code',
        effort: 'medium',
        offset: 30,
      },
    });
  });

  it('treats an equal offset as a duplicate and preserves the existing value', () => {
    const existing: RunEventAssignments = {
      'execution-1': {
        stepExecutionId: 'execution-1',
        agentKind: 'codex',
        effort: 'high',
        offset: 7,
      },
    };

    const result = reconcileRunEventAssignments(existing, [
      spawned(7, { step_execution_id: 'execution-1', agent_kind: 'opencode', effort: 'max' }),
    ]);

    expect(result).toBe(existing);
    expect(result['execution-1']).toBe(existing['execution-1']);
  });

  it('replaces the model when a later spawn for the same execution pins another', () => {
    const earlier = reconcileRunEventAssignments({}, [
      spawned(10, {
        step_execution_id: 'execution-1',
        agent_kind: 'claude-code',
        effort: 'high',
        model: 'claude-sonnet-4-5',
      }),
    ]);

    expect(reconcileRunEventAssignments(earlier, [
      spawned(11, {
        step_execution_id: 'execution-1',
        agent_kind: 'claude-code',
        effort: 'high',
        model: null,
      }),
    ])['execution-1']).toMatchObject({ model: null, offset: 11 });
  });

  it('isolates executions and skips invalid events', () => {
    expect(reconcileRunEventAssignments({}, [
      spawned(3, { step_execution_id: 'execution-a', agent_kind: 'codex', effort: 'low' }),
      spawned(4, { step_execution_id: 'execution-b', agent_kind: 'hermes', effort: null }),
      spawned(99, { step_execution_id: 'execution-a', agent_kind: 'codex' }),
    ])).toMatchObject({
      'execution-a': { agentKind: 'codex', effort: 'low', offset: 3 },
      'execution-b': { agentKind: 'hermes', effort: null, offset: 4 },
    });
  });
});

describe('assignmentEffortLabel', () => {
  it.each(EFFORT_LEVELS)('uses the canonical label for %s', (effort) => {
    expect(assignmentEffortLabel(effort)).toBe(EFFORT_LABELS[effort]);
  });

  it('uses explicit neutral wording for no injected effort', () => {
    expect(assignmentEffortLabel(null)).toBe(NO_INJECTED_EFFORT_LABEL);
    expect(NO_INJECTED_EFFORT_LABEL).toBe('No injected effort');
  });
});

describe('assignmentModelLabel', () => {
  it('names the harness default when no model was pinned', () => {
    expect(assignmentModelLabel(null)).toBe(HARNESS_DEFAULT_MODEL_LABEL);
    expect(HARNESS_DEFAULT_MODEL_LABEL).toBe('Harness default');
  });

  it('passes a pinned model id through unchanged', () => {
    expect(assignmentModelLabel('gpt-5.6-codex')).toBe('gpt-5.6-codex');
  });
});

describe('assignmentAriaLabel', () => {
  it('keeps the agent and effort reading when no model label is given', () => {
    expect(assignmentAriaLabel('Research', 'codex', 'High')).toBe(
      'Actual assignment for Research: Agent: codex; Effective effort: High',
    );
  });

  it('places the model between agent and effort when given', () => {
    expect(assignmentAriaLabel('Research', 'claude-code', 'High', 'Harness default')).toBe(
      'Actual assignment for Research: Agent: claude-code; Model: Harness default; Effective effort: High',
    );
  });
});
