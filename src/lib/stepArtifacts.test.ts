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

import { listStepArtifacts } from './stepArtifacts';
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
});
