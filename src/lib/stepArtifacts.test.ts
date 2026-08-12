/**
 * The claim: an agent step's declared paths are a changed-files list, and the
 * rule that folds them is arithmetic the rendering surface never re-decides.
 *
 * Both directions are user-visible. Listing all of them buries the one document
 * the agent wrote to be read under every file it touched; folding them on a
 * non-agent step hides the only output that step has. The hidden count is
 * asserted against the *deduped* total because a duplicate counted as a hidden
 * file reports work that never happened.
 */
import { describe, expect, it } from 'vitest';

import { listReviewableGateArtifacts, listStepArtifacts } from './stepArtifacts';
import type { StepExecution } from '../types';

function step(over: Partial<StepExecution> = {}): StepExecution {
  return {
    id: 'se-1',
    feature_id: 'f-1',
    step_id: 's-implement',
    step_index: 0,
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: [],
    created_at: 0,
    updated_at: 0,
    ...over,
  };
}

describe('listStepArtifacts', () => {
  it('keeps only the markdown an agent step wrote and counts the rest away', () => {
    const result = listStepArtifacts(
      step({
        artifact_paths: [
          'artifacts/report.md',
          'src/lib/auth.ts',
          'src/lib/auth.test.ts',
          'Cargo.toml',
        ],
      }),
    );
    expect(result.listed).toEqual(['artifacts/report.md']);
    expect(result.hiddenCount).toBe(3);
  });

  it('lists every path a non-agent step declared', () => {
    const result = listStepArtifacts(
      step({ step_kind: 'gate', artifact_paths: ['artifacts/verdict.json', 'diff.patch'] }),
    );
    expect(result.listed).toEqual(['artifacts/verdict.json', 'diff.patch']);
    expect(result.hiddenCount).toBe(0);
  });

  it('dedupes before deciding, so a repeated path is not a hidden file', () => {
    const result = listStepArtifacts(
      step({
        artifact_paths: ['artifacts/plan.md', 'artifacts/plan.md', 'src/a.ts', 'src/a.ts'],
      }),
    );
    expect(result.listed).toEqual(['artifacts/plan.md']);
    expect(result.hiddenCount).toBe(1);
  });

  it('reads the legacy single artifact_path when no list was declared', () => {
    const result = listStepArtifacts(
      step({ step_kind: 'gate', artifact_paths: [], artifact_path: 'artifacts/plan.md' }),
    );
    expect(result.listed).toEqual(['artifacts/plan.md']);
  });

  it('answers empty for a node with no execution behind it', () => {
    expect(listStepArtifacts(null)).toEqual({ listed: [], hiddenCount: 0 });
  });

  it('reports an agent step whose every path was folded away', () => {
    const result = listStepArtifacts(step({ artifact_paths: ['src/a.ts', 'src/b.ts'] }));
    expect(result.listed).toEqual([]);
    expect(result.hiddenCount).toBe(2);
  });

  it('lists a ticket list among an agent step\'s source edits', () => {
    // `s-tickets` declares this and nothing else, so folding it leaves the
    // step with no rows anywhere it is listed — the gate picker included.
    const result = listStepArtifacts(
      step({ step_id: 's-tickets', artifact_paths: ['artifacts/task-list.json', 'src/a.ts'] }),
    );
    expect(result.listed).toEqual(['artifacts/task-list.json']);
    expect(result.hiddenCount).toBe(1);
  });

  it('still folds a plain JSON artifact an agent step wrote', () => {
    const result = listStepArtifacts(step({ artifact_paths: ['package-lock.json'] }));
    expect(result.listed).toEqual([]);
    expect(result.hiddenCount).toBe(1);
  });
});

describe('listReviewableGateArtifacts', () => {
  const research = step({ id: 'se-1', step_id: 's-research', step_index: 1, artifact_paths: ['artifacts/r.md'] });
  const tickets = step({ id: 'se-2', step_id: 's-tickets', step_index: 3, artifact_paths: ['artifacts/task-list.json'] });
  const baseline = step({ id: 'se-0', step_id: 's-baseline', step_index: 0, step_kind: 'command', artifact_paths: [] });

  it('returns one group per predecessor with something listable, in step order', () => {
    const groups = listReviewableGateArtifacts([tickets, baseline, research], 4);
    expect(groups.map((g) => g.step.step_id)).toEqual(['s-research', 's-tickets']);
    expect(groups[1].listed).toEqual(['artifacts/task-list.json']);
  });

  it('excludes the gate step itself and everything after it', () => {
    expect(listReviewableGateArtifacts([research, tickets], 3).map((g) => g.step.step_id)).toEqual([
      's-research',
    ]);
  });

  it('excludes an earlier gate, whose paths are its own predecessor\'s copied verbatim', () => {
    const firstGate = step({
      id: 'se-3',
      step_id: 's-gate-review',
      step_index: 4,
      step_kind: 'gate',
      artifact_paths: ['artifacts/task-list.json'],
    });
    const groups = listReviewableGateArtifacts([research, tickets, firstGate], 5);
    expect(groups.map((g) => g.step.step_id)).toEqual(['s-research', 's-tickets']);
  });

  it('is empty when no predecessor has anything listable', () => {
    expect(listReviewableGateArtifacts([baseline], 1)).toEqual([]);
  });
});
