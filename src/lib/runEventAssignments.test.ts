import { describe, expect, it } from 'vitest';

import { EFFORT_LABELS, EFFORT_LEVELS } from './effortLevels';
import {
  assignmentEffortLabel,
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
      offset: 12,
    });
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
