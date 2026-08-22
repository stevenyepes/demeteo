import { describe, expect, it } from 'vitest';

import { describePullRequestRow } from './pullRequestRow';
import type { PullRequestSummary } from './pullRequests';

const NOW = Date.parse('2026-08-15T09:00:00Z');

function summary(overrides: Partial<PullRequestSummary> = {}): PullRequestSummary {
  return {
    number: 412,
    title: 'Tighten the refspec guard',
    author: 'octocat',
    source_branch: 'patch-1',
    target_branch: 'main',
    draft: false,
    web_url: 'https://github.com/acme/app/pull/412',
    created_at: '2026-08-12T09:00:00Z',
    updated_at: '2026-08-15T07:00:00Z',
    head_repo_path: 'octocat/app',
    head_fetch_spec: 'refs/pull/412/head',
    from_fork: true,
    maintainer_can_modify: false,
    head_repo_push: false,
    ...overrides,
  };
}

describe('describePullRequestRow', () => {
  it('maps a fully described pull request onto the three tiers', () => {
    const row = describePullRequestRow(
      summary({
        draft: true,
        additions: 120,
        deletions: 8,
        changed_files: 3,
        has_conflicts: true,
        review_status: 'running',
      }),
      NOW,
    );

    expect(row.number).toBe('#412');
    expect(row.branchLabel).toBe('patch-1 → main');
    expect(row.updatedAgo).toBe('2h ago');
    expect(row.timeline).toBe('opened 3d ago · updated 2h ago');
    expect(row.diffstat).toEqual({ additions: '+120', deletions: '−8' });
    expect(row.fileLabel).toBe('3 files');
    expect(row.chips).toEqual([
      { label: 'Draft', tone: 'slate' },
      { label: 'Conflicts', tone: 'amber' },
      { label: 'Running', tone: 'cyan' },
    ]);
  });

  it('omits the tiers a list endpoint did not carry', () => {
    const row = describePullRequestRow(summary({}), NOW);

    expect(row.diffstat).toBeNull();
    expect(row.fileLabel).toBeNull();
    expect(row.chips).toEqual([]);
  });

  it('says so when the provider has not finished deciding mergeability', () => {
    const row = describePullRequestRow(summary({ has_conflicts: null }), NOW);

    expect(row.chips).toEqual([{ label: 'Merge unknown', tone: 'slate' }]);
  });

  it('says nothing about a request the provider called clean', () => {
    const row = describePullRequestRow(summary({ has_conflicts: false }), NOW);

    expect(row.chips).toEqual([]);
  });

  it('counts one changed file in the singular', () => {
    expect(describePullRequestRow(summary({ changed_files: 1 }), NOW).fileLabel).toBe('1 file');
  });

  it('drops a timestamp it cannot read rather than dating it to the epoch', () => {
    const row = describePullRequestRow(summary({ created_at: 'never' }), NOW);

    expect(row.timeline).toBe('updated 2h ago');
  });
});
