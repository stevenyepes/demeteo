import { describe, expect, it } from 'vitest';

import { reconcileSteps } from './stepReconcile';
import type { StepExecution } from '../types';

/**
 * Typed as `Required` so a new `StepExecution` field breaks this fixture, which
 * is what makes the "every field is compared" test below stay exhaustive.
 */
const FULL: Required<StepExecution> = {
  id: 'se-1',
  feature_id: 'f-1',
  step_id: 'implement',
  step_index: 2,
  step_kind: 'agent',
  status: 'running',
  cost_usd: 0.42,
  tokens: 1200,
  wall_clock_secs: 31,
  artifact_path: 'out/plan.md',
  artifact_paths: ['out/plan.md'],
  error_message: 'boom',
  iteration_count: 1,
  created_at: 1_700_000_000,
  updated_at: 1_700_000_100,
  cache_read_input_tokens: 900,
  cache_creation_input_tokens: 120,
};

function step(overrides: Partial<StepExecution> & { id: string }): StepExecution {
  return { ...FULL, ...overrides };
}

/** A fresh row carrying identical values, as a reload builds it off the wire. */
function clone(row: StepExecution): StepExecution {
  return { ...row, artifact_paths: [...row.artifact_paths] };
}

function perturb(value: unknown): unknown {
  if (Array.isArray(value)) return [...value, 'extra'];
  if (typeof value === 'number') return value + 1;
  if (typeof value === 'string') return `${value}-changed`;
  throw new Error(`fixture field has no perturbation for ${typeof value}`);
}

function withChangedField(key: keyof StepExecution): StepExecution {
  const row: Record<string, unknown> = { ...FULL };
  row[key] = perturb(FULL[key]);
  return row as unknown as StepExecution;
}

describe('reconcileSteps', () => {
  it('returns the previous array itself when nothing changed', () => {
    const prev = [step({ id: 'a' }), step({ id: 'b', step_index: 3 })];
    const next = prev.map(clone);

    expect(reconcileSteps(prev, next)).toBe(prev);
  });

  it('returns the previous array for two empty lists', () => {
    const prev: StepExecution[] = [];

    expect(reconcileSteps(prev, [])).toBe(prev);
  });

  it('gives only the changed row a new identity', () => {
    const prev = [step({ id: 'a' }), step({ id: 'b' }), step({ id: 'c' })];
    const next = [clone(prev[0]), { ...clone(prev[1]), status: 'completed' }, clone(prev[2])];

    const merged = reconcileSteps(prev, next);

    expect(merged).not.toBe(prev);
    expect(merged[0]).toBe(prev[0]);
    expect(merged[1]).toBe(next[1]);
    expect(merged[2]).toBe(prev[2]);
  });

  it('reuses the surviving rows when a row is appended', () => {
    const prev = [step({ id: 'a' }), step({ id: 'b' })];
    const next = [clone(prev[0]), clone(prev[1]), step({ id: 'c' })];

    const merged = reconcileSteps(prev, next);

    expect(merged).toHaveLength(3);
    expect(merged[0]).toBe(prev[0]);
    expect(merged[1]).toBe(prev[1]);
    expect(merged[2]).toBe(next[2]);
  });

  it('reuses the surviving rows when a row is removed', () => {
    const prev = [step({ id: 'a' }), step({ id: 'b' }), step({ id: 'c' })];
    const next = [clone(prev[0]), clone(prev[2])];

    const merged = reconcileSteps(prev, next);

    expect(merged).toEqual([prev[0], prev[2]]);
    expect(merged[0]).toBe(prev[0]);
    expect(merged[1]).toBe(prev[2]);
  });

  it('reuses every row when the order changed but the rows did not', () => {
    const prev = [step({ id: 'a' }), step({ id: 'b' })];
    const next = [clone(prev[1]), clone(prev[0])];

    const merged = reconcileSteps(prev, next);

    expect(merged).not.toBe(prev);
    expect(merged[0]).toBe(prev[1]);
    expect(merged[1]).toBe(prev[0]);
  });

  it('matches rows by id, not by position', () => {
    const prev = [step({ id: 'a', status: 'completed' }), step({ id: 'b', status: 'running' })];
    const next = [clone(prev[1]), { ...clone(prev[0]), status: 'failed' }];

    const merged = reconcileSteps(prev, next);

    expect(merged[0]).toBe(prev[1]);
    expect(merged[1]).toBe(next[1]);
  });

  it('compares artifact_paths by content, not by reference', () => {
    const prev = [step({ id: 'a', artifact_paths: ['one.md', 'two.md'] })];

    expect(reconcileSteps(prev, [step({ id: 'a', artifact_paths: ['one.md', 'two.md'] })])).toBe(
      prev,
    );
    expect(reconcileSteps(prev, [step({ id: 'a', artifact_paths: ['two.md', 'one.md'] })])).not.toBe(
      prev,
    );
    expect(reconcileSteps(prev, [step({ id: 'a', artifact_paths: ['one.md'] })])).not.toBe(prev);
  });

  it('treats a missing optional field as equal to an explicit null', () => {
    const withNull = step({ id: 'a', error_message: null });
    const prev = [withNull];
    const absent: Record<string, unknown> = { ...withNull };
    delete absent.error_message;

    expect(reconcileSteps(prev, [absent as unknown as StepExecution])).toBe(prev);
  });

  it('detects a change in every field of a fully populated row', () => {
    const prev = [FULL as StepExecution];

    for (const key of Object.keys(FULL) as (keyof StepExecution)[]) {
      const next = [withChangedField(key)];
      const merged = reconcileSteps(prev, next);

      if (key === 'id') {
        expect(merged[0], `field ${key}`).toBe(next[0]);
        continue;
      }
      expect(merged, `field ${key}`).not.toBe(prev);
      expect(merged[0], `field ${key}`).toBe(next[0]);
    }
  });

  it('leaves both inputs untouched', () => {
    const prev = [step({ id: 'a' })];
    const next = [{ ...clone(prev[0]), status: 'failed' }];
    const prevSnapshot = prev.slice();

    reconcileSteps(prev, next);

    expect(prev).toEqual(prevSnapshot);
    expect(next[0].status).toBe('failed');
  });
});
