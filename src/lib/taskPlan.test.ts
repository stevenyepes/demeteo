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
});
