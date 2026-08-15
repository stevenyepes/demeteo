// The three shapes that reach this mapper are the three the launch has to
// survive: a same-repo pull request, one whose head lives in a fork this clone
// has no remote for, and a GitLab merge request. All three fetch a ref in the
// upstream repository, and none of them may fetch the branch name the provider
// displays.

import { describe, expect, it } from 'vitest';

import { planReviewLaunch, REVIEW_STARTER_WORKFLOW_ID } from './reviewLaunch';
import type { PullRequestSummary } from './pullRequests';

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
    head_repo_path: 'acme/app',
    head_fetch_spec: 'refs/pull/412/head',
    from_fork: false,
    maintainer_can_modify: false,
    head_repo_push: true,
    ...overrides,
  };
}

function launchOf(pullRequest: PullRequestSummary, instructions?: string) {
  const plan = planReviewLaunch(pullRequest, instructions);
  if (!plan.ok) throw new Error(`refused: ${plan.reason}`);
  return plan.launch;
}

describe('planReviewLaunch', () => {
  it('fetches a same-repo pull request by its upstream head ref, not by its branch', () => {
    const launch = launchOf(summary());

    expect(launch.origin).toEqual({
      kind: 'ref',
      fetch_spec: 'refs/pull/412/head',
      label: 'patch-1',
    });
    expect(launch.diffBaseBranch).toBe('main');
    expect(launch.workflowId).toBe(REVIEW_STARTER_WORKFLOW_ID);
  });

  it('fetches a fork pull request the same way, and measures it against the upstream target', () => {
    const launch = launchOf(
      summary({
        number: 9,
        source_branch: 'patch-1',
        target_branch: 'release/2.1',
        head_repo_path: 'contributor/app',
        head_fetch_spec: 'refs/pull/9/head',
        from_fork: true,
      }),
    );

    expect(launch.origin).toEqual({
      kind: 'ref',
      fetch_spec: 'refs/pull/9/head',
      label: 'patch-1',
    });
    expect(launch.diffBaseBranch).toBe('release/2.1');
    expect(launch.description).toContain('Head: patch-1 (from a fork) → base: release/2.1');
  });

  it('fetches a GitLab merge request from its own head namespace', () => {
    const launch = launchOf(
      summary({
        number: 77,
        source_branch: 'feature/parser',
        target_branch: 'develop',
        head_repo_path: null,
        head_fetch_spec: 'refs/merge-requests/77/head',
        from_fork: true,
        web_url: 'https://gitlab.example.com/acme/app/-/merge_requests/77',
      }),
    );

    expect(launch.origin).toEqual({
      kind: 'ref',
      fetch_spec: 'refs/merge-requests/77/head',
      label: 'feature/parser',
    });
    expect(launch.diffBaseBranch).toBe('develop');
  });

  it('refuses a summary whose head ref did not survive the wire', () => {
    for (const spec of ['', '   ', 'patch-1', '--upload-pack=touch X', 'refs/pull/1/head extra']) {
      const plan = planReviewLaunch(summary({ head_fetch_spec: spec }));
      expect(plan.ok, spec).toBe(false);
      if (!plan.ok) expect(plan.reason).toBe('no-head-ref');
    }
  });

  it('refuses a summary that names no branch to measure the change against', () => {
    const plan = planReviewLaunch(summary({ target_branch: '  ' }));

    expect(plan.ok).toBe(false);
    if (!plan.ok) expect(plan.reason).toBe('no-base-branch');
  });

  it('keeps the number when the title has to give way', () => {
    const launch = launchOf(
      summary({
        number: 412,
        title:
          'Tighten the refspec guard so a fetched origin cannot be read as an option by git',
      }),
    );

    expect(launch.title.length).toBeLessThanOrEqual(72);
    expect(launch.title).toBe('Review PR #412 — Tighten the refspec guard so a fetched origin cannot…');
  });

  it('carries the request identity and the extra instructions into the description', () => {
    const launch = launchOf(summary(), '  Concentrate on the auth changes.  ');

    expect(launch.description).toContain('Pull request #412: Tighten the refspec guard');
    expect(launch.description).toContain('https://github.com/acme/app/pull/412');
    expect(launch.description).toContain('Author: octocat');
    expect(launch.description).toContain('Extra instructions:\nConcentrate on the auth changes.');
  });

  it('leaves no empty instructions heading behind when none were given', () => {
    expect(launchOf(summary(), '   ').description).not.toContain('Extra instructions');
  });
});
