// Unit tests for `src/lib/taskPlan.ts`.

import { describe, it, expect } from 'vitest';
import { isTaskPlan, parseTaskPlan, findPlanIssues } from './taskPlan';
import type { TaskPlan } from '../types';

const validPayload = {
  kind: 'greenfield',
  tasks: [
    { id: 't1', title: 'First task', description: 'Do the first thing' },
    { id: 't2', title: 'Second task', description: 'Do the second thing', blocked_by: ['t1'] },
  ],
};

describe('isTaskPlan', () => {
  it('accepts a gate-review payload with no `cycle` field', () => {
    expect(isTaskPlan(validPayload)).toBe(true);
  });

  it('rejects the legacy `subtasks` shape', () => {
    expect(isTaskPlan({ kind: 'greenfield', subtasks: [{ id: 't1', title: 'x', description: 'y' }] })).toBe(false);
  });

  it('rejects a task missing a required string field', () => {
    expect(isTaskPlan({ tasks: [{ id: 't1', title: 'x' }] })).toBe(false);
  });

  it('rejects non-object input', () => {
    expect(isTaskPlan(null)).toBe(false);
    expect(isTaskPlan('not a plan')).toBe(false);
  });

  it('rejects a task whose `files` is a scalar string instead of an array', () => {
    expect(
      isTaskPlan({ tasks: [{ id: 't1', title: 'x', description: 'y', files: 'src/foo.ts' }] })
    ).toBe(false);
  });

  it('rejects a task whose `acceptance` is a scalar string instead of an array', () => {
    expect(
      isTaskPlan({ tasks: [{ id: 't1', title: 'x', description: 'y', acceptance: 'looks right' }] })
    ).toBe(false);
  });

  it('rejects a task whose `blocked_by` is a scalar string instead of an array', () => {
    expect(
      isTaskPlan({ tasks: [{ id: 't1', title: 'x', description: 'y', blocked_by: 't0' }] })
    ).toBe(false);
  });

  it('rejects a task whose `files` array contains a non-string element', () => {
    expect(
      isTaskPlan({ tasks: [{ id: 't1', title: 'x', description: 'y', files: ['src/foo.ts', 1] }] })
    ).toBe(false);
  });

  it('rejects a task whose `test_command` is a number', () => {
    expect(
      isTaskPlan({ tasks: [{ id: 't1', title: 'x', description: 'y', test_command: 42 }] })
    ).toBe(false);
  });

  it('accepts a task whose `test_command`/`retry_note` are explicitly null', () => {
    expect(
      isTaskPlan({
        tasks: [{ id: 't1', title: 'x', description: 'y', test_command: null, retry_note: null }],
      })
    ).toBe(true);
  });

  // Every rejection below is a field the renderer dereferences unguarded, so
  // a `true` verdict here is a TypeError there and — with no ErrorBoundary in
  // `src/` — a blank window at a gate awaiting a decision.
  it('rejects a plan whose `notes` is an object rather than a string', () => {
    expect(isTaskPlan({ tasks: [], notes: { reason: 'x' } })).toBe(false);
  });

  it('accepts a plan whose `notes` is explicitly null', () => {
    expect(isTaskPlan({ tasks: [], notes: null })).toBe(true);
  });

  it('rejects a plan whose `history` is an object rather than an array', () => {
    expect(isTaskPlan({ tasks: [], history: { cycle: 0 } })).toBe(false);
  });

  it('rejects a history entry with no `tasks` array', () => {
    expect(isTaskPlan({ tasks: [], history: [{ cycle: 0, kind: 'greenfield' }] })).toBe(false);
  });

  it('rejects a history entry whose `kind` is not a PlanKind', () => {
    expect(isTaskPlan({ tasks: [], history: [{ cycle: 0, kind: 'Greenfield', tasks: [] }] })).toBe(false);
  });

  it('accepts a well-formed history entry', () => {
    expect(
      isTaskPlan({
        kind: 'rework',
        cycle: 1,
        tasks: [],
        history: [{ cycle: 0, kind: 'greenfield', tasks: [{ id: 't1', title: 'x', description: 'y' }] }],
      })
    ).toBe(true);
  });

  it('rejects a plan whose `kind` is not a PlanKind', () => {
    expect(isTaskPlan({ tasks: [], kind: 'brownfield' })).toBe(false);
  });

  it('rejects a plan whose `cycle` is a string', () => {
    expect(isTaskPlan({ tasks: [], cycle: '1' })).toBe(false);
  });

  it('accepts a task with well-typed optional fields', () => {
    expect(
      isTaskPlan({
        tasks: [
          {
            id: 't1',
            title: 'x',
            description: 'y',
            files: ['src/foo.ts'],
            acceptance: ['it works'],
            blocked_by: ['t0'],
            test_command: 'npm test',
            retry_note: 'retried once',
          },
        ],
      })
    ).toBe(true);
  });
});

describe('parseTaskPlan', () => {
  it('returns the parsed plan for valid JSON', () => {
    expect(parseTaskPlan(JSON.stringify(validPayload))).toEqual(validPayload);
  });

  it('returns null on malformed JSON', () => {
    expect(parseTaskPlan('{not valid json')).toBeNull();
  });

  it('returns null for the legacy `subtasks` payload', () => {
    expect(parseTaskPlan(JSON.stringify({ subtasks: [{ id: 't1', title: 'x', description: 'y' }] }))).toBeNull();
  });

  it('returns null when `files` is a scalar string instead of an array', () => {
    expect(
      parseTaskPlan(JSON.stringify({ tasks: [{ id: 't1', title: 'x', description: 'y', files: 'src/foo.ts' }] }))
    ).toBeNull();
  });
});

describe('findPlanIssues', () => {
  it('returns [] for a clean plan', () => {
    const plan: TaskPlan = { kind: 'greenfield', cycle: 1, tasks: [
      { id: 't1', title: 'First', description: 'desc' },
      { id: 't2', title: 'Second', description: 'desc', blocked_by: ['t1'] },
    ] };
    expect(findPlanIssues(plan)).toEqual([]);
  });

  it('flags a duplicate task id', () => {
    const plan: TaskPlan = { kind: 'greenfield', cycle: 1, tasks: [
      { id: 't1', title: 'First', description: 'desc' },
      { id: 't1', title: 'Duplicate', description: 'desc' },
    ] };
    const issues = findPlanIssues(plan);
    expect(issues.length).toBeGreaterThan(0);
    expect(issues.some((issue) => issue.includes('t1'))).toBe(true);
  });

  it('flags a self-referential blocked_by entry', () => {
    const plan: TaskPlan = { kind: 'greenfield', cycle: 1, tasks: [
      { id: 't1', title: 'First', description: 'desc', blocked_by: ['t1'] },
    ] };
    const issues = findPlanIssues(plan);
    expect(issues.length).toBeGreaterThan(0);
    expect(issues.some((issue) => issue.includes('t1'))).toBe(true);
  });

  // The three below are the rules the Rust `validate_task_plan` fails the
  // sequence step on, non-retryably, *after* the gate was approved. Missing
  // them here is a plan shown clean and then refused.
  it('flags a blocked_by naming a task that does not exist', () => {
    const plan: TaskPlan = { tasks: [
      { id: 't1', title: 'First', description: 'desc', blocked_by: ['t9'] },
    ] };
    expect(findPlanIssues(plan)).toEqual([
      'Task t1 is blocked by t9, which is not an earlier task in the list',
    ]);
  });

  it('flags a blocked_by naming a task declared later in the list', () => {
    const plan: TaskPlan = { tasks: [
      { id: 't1', title: 'First', description: 'desc', blocked_by: ['t2'] },
      { id: 't2', title: 'Second', description: 'desc' },
    ] };
    expect(findPlanIssues(plan)).toEqual([
      'Task t1 is blocked by t2, which is not an earlier task in the list',
    ]);
  });

  it('flags a blank task id', () => {
    const plan: TaskPlan = { tasks: [{ id: '   ', title: 'First', description: 'desc' }] };
    expect(findPlanIssues(plan)).toEqual(['Task at position 1 has an empty id']);
  });

  it('reports every violation rather than stopping at the first', () => {
    const plan: TaskPlan = { tasks: [
      { id: 't1', title: 'First', description: 'desc', blocked_by: ['nope'] },
      { id: 't1', title: 'Dup', description: 'desc' },
    ] };
    expect(findPlanIssues(plan)).toHaveLength(2);
  });

  it('ignores a blank blocked_by entry, as the executor does', () => {
    const plan: TaskPlan = { tasks: [
      { id: 't1', title: 'First', description: 'desc', blocked_by: ['', '  '] },
    ] };
    expect(findPlanIssues(plan)).toEqual([]);
  });

  it('does not iterate a scalar blocked_by string per character', () => {
    const plan = { tasks: [
      { id: 't1', title: 'First', description: 'desc', blocked_by: 'abc' },
    ] } as unknown as TaskPlan;
    expect(findPlanIssues(plan)).toEqual([]);
  });
});
