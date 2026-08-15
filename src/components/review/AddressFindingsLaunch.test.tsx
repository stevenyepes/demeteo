// The join this surface rests on is not persisted anywhere: it recovers the
// reviewed pull request by matching the run's origin fetch spec against the
// open-request listing. That is the part worth holding — a wrong match opens a
// pull request against a stranger's branch, and nothing about the screen would
// say so.

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { AddressFindingsLaunch } from './AddressFindingsLaunch';
import type { StepExecution } from '../../types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const REPORT = '1. The refspec guard admits a leading dash.';

const STEPS = [
  {
    id: 'se-1',
    feature_id: 'feat-1',
    step_id: 's-review',
    step_index: 0,
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: ['/tmp/wt/artifacts/code-review.md'],
    created_at: 0,
    updated_at: 0,
  },
] as unknown as StepExecution[];

function pullRequest(over: Record<string, unknown> = {}) {
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

/** Rejects anything it was not told to answer: a stub that resolves every
 *  command would let this suite pass against a component reading the wrong one. */
function backend(input: {
  fetchSpec?: string;
  pullRequests?: Record<string, unknown>[];
  report?: string;
}) {
  const launches: Record<string, unknown>[] = [];
  vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
    if (cmd === 'feature_get') {
      return Promise.resolve(
        input.fetchSpec === undefined
          ? { id: 'feat-1' }
          : { id: 'feat-1', origin: { kind: 'ref', fetch_spec: input.fetchSpec, label: 'patch-1' } },
      );
    }
    if (cmd === 'list_open_pull_requests') {
      return Promise.resolve(input.pullRequests ?? [pullRequest()]);
    }
    if (cmd === 'get_proposed_strategy') {
      return Promise.resolve({ worktree_strategy: { default_branch: 'main' } });
    }
    if (cmd === 'artifact_body') return Promise.resolve(input.report ?? REPORT);
    if (cmd === 'start_feature') {
      launches.push(typeof args === 'object' && args !== null ? { ...args } : {});
      return Promise.resolve({ id: 'feat-2', title: 'fix', status: 'running' });
    }
    return Promise.reject(new Error(`unexpected command: ${cmd}`));
  });
  return launches;
}

function mount(onLaunch: (params: never) => Promise<void>) {
  render(
    <AddressFindingsLaunch
      featureId="feat-1"
      projectId="proj-1"
      steps={STEPS}
      onLaunch={onLaunch as never}
    />,
  );
}

describe('AddressFindingsLaunch', () => {
  it('launches against the reviewed head branch, behind a confirmation', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: Record<string, unknown>[] = [];
    mount(async (params) => {
      launched.push(params as unknown as Record<string, unknown>);
    });

    await userEvent.click(await screen.findByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].workflowId).toBe('wf-starter-address-review');
    // The whole point of the feature: the fix stacks on the branch that was
    // reviewed, so a human reads it against the work it fixes.
    expect(launched[0].origin).toEqual({ kind: 'branch', base: 'patch-1' });
    expect(launched[0].diffBaseBranch).toBe('patch-1');
    expect(String(launched[0].description)).toContain(REPORT);
  });

  it('does nothing at all until the user confirms', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: unknown[] = [];
    mount(async (params) => {
      launched.push(params);
    });

    await userEvent.click(await screen.findByTestId('address-findings'));
    expect(launched).toHaveLength(0);
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(launched).toHaveLength(0);
  });

  // The failure this test exists for is silent: a mismatched join renders the
  // *wrong* pull request's branches with total confidence.
  it('renders nothing when no open request matches the run origin', async () => {
    backend({ fetchSpec: 'refs/pull/999/head' });
    mount(async () => {});

    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalled());
    expect(screen.queryByTestId('address-findings')).not.toBeInTheDocument();
  });

  it('renders nothing for a run that is not a review', async () => {
    backend({});
    mount(async () => {});

    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalled());
    expect(screen.queryByTestId('address-findings')).not.toBeInTheDocument();
  });

  it('refuses, in place, a destination the launch arguments cannot express', async () => {
    backend({
      fetchSpec: 'refs/pull/412/head',
      pullRequests: [pullRequest({ from_fork: true, target_branch: 'release/2.x' })],
    });
    mount(async () => {});

    expect(await screen.findByTestId('fix-refused')).toHaveTextContent('release/2.x');
    expect(screen.getByTestId('address-findings')).toBeDisabled();
  });
});
