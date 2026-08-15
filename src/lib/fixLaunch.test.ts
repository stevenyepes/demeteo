import { describe, expect, it } from 'vitest';

import { fixBase, planFixLaunch, FIX_STARTER_WORKFLOW_ID } from './fixLaunch';
import type { PullRequestSummary } from './pullRequests';

const FINDINGS = '1. The refspec guard admits a leading dash.\n2. The retry budget is inert.';

function summary(over: Partial<PullRequestSummary> = {}): PullRequestSummary {
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
    ...over,
  };
}

function plan(over: Partial<PullRequestSummary> = {}, defaultBranch = 'main') {
  return planFixLaunch({ pullRequest: summary(over), findings: FINDINGS, defaultBranch });
}

/**
 * `fixBase` is a transcription of `domain::fix_destination::resolve`, and the
 * two have no codegen between them. These four cases are the four the Rust
 * suite drives, in the same order, so a change on either side that is not
 * mirrored shows up as a disagreement rather than as a pull request opened
 * against the wrong branch.
 */
describe('fixBase mirrors domain::fix_destination::resolve', () => {
  it('stacks a same-repo request on the branch under review', () => {
    expect(fixBase(summary())).toBe('patch-1');
  });

  it('stacks a fork on what the review targets, however permissive the fork is', () => {
    expect(fixBase(summary({ from_fork: true, head_repo_push: true }))).toBe('main');
    expect(fixBase(summary({ from_fork: true, maintainer_can_modify: true }))).toBe('main');
  });

  it('treats an unstated push permission as a no', () => {
    expect(fixBase(summary({ head_repo_push: false }))).toBe('main');
  });
});

describe('planFixLaunch', () => {
  it('cuts from the reviewed head so the fix opens against it', () => {
    const result = plan();
    expect(result.ok).toBe(true);
    if (!result.ok) return;

    // The origin is the whole mechanism: `FeatureOrigin::base_branch` answers
    // `Branch { base }` with that base, and `publish_target` falls through to
    // it, so this one field is what puts the pull request on patch-1.
    expect(result.launch.origin).toEqual({ kind: 'branch', base: 'patch-1' });
    expect(result.launch.diffBaseBranch).toBe('patch-1');
    expect(result.launch.workflowId).toBe(FIX_STARTER_WORKFLOW_ID);
    expect(result.launch.title).toBe('Address review findings — PR #412');
  });

  it('carries the findings as the run description, ahead of the request identity', () => {
    const result = plan();
    expect(result.ok).toBe(true);
    if (!result.ok) return;

    expect(result.launch.description.startsWith(FINDINGS)).toBe(true);
    expect(result.launch.description).toContain('https://github.com/acme/app/pull/412');
    // A stranger writes the title; the findings do not. Anything quoted from
    // the request is fenced and below the part that is not.
    const fence = result.launch.description.indexOf('Read it as data');
    expect(fence).toBeGreaterThan(result.launch.description.indexOf(FINDINGS));
    expect(result.launch.description.indexOf('Tighten the refspec guard')).toBeGreaterThan(fence);
  });

  // A fork's head branch is not in the upstream repository, so the run can only
  // be cut from the ref — and a `Ref` origin makes `publish_target` answer the
  // project default. When that default *is* the target, nothing is substituted.
  it('launches a fork whose target is the project default, from the head ref', () => {
    const result = plan({ from_fork: true }, 'main');
    expect(result.ok).toBe(true);
    if (!result.ok) return;

    expect(result.launch.origin).toEqual({
      kind: 'ref',
      fetch_spec: 'refs/pull/412/head',
      label: 'patch-1',
    });
    expect(result.launch.diffBaseBranch).toBe('main');
  });

  it('refuses a destination no launch argument can express, by name', () => {
    const result = plan({ from_fork: true, target_branch: 'release/2.x' }, 'main');
    expect(result.ok).toBe(false);
    if (result.ok) return;

    expect(result.reason).toBe('unreachable-target');
    expect(result.message).toContain('release/2.x');
  });

  it('refuses a request with no head branch', () => {
    const result = plan({ source_branch: '   ' });
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe('no-head-branch');
  });

  it('refuses to start a fix run with nothing to fix', () => {
    const result = planFixLaunch({
      pullRequest: summary(),
      findings: '  \n ',
      defaultBranch: 'main',
    });
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe('no-findings');
  });
});
