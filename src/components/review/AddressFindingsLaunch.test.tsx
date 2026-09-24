// The join this surface rests on is not persisted anywhere: it recovers the
// reviewed pull request by matching the run's origin fetch spec against the
// open-request listing. That is the part worth holding — a wrong match opens a
// pull request against a stranger's branch, and nothing about the screen would
// say so.

import type { ReactElement } from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { AddressFindingsLaunch } from './AddressFindingsLaunch';
import { FORK_EXPOSURE } from './PullRequestLaunch';
import { FIX_STARTER_WORKFLOW_ID, planFixLaunch, type FixLaunchParams } from '../../lib/fixLaunch';
import type { PullRequestSummary } from '../../lib/pullRequests';
import { GATE_FENCE_CLOSE, GATE_FENCE_OPEN, GATE_HEADING } from '../../lib/reviewEvidence';
import type { StepConfig, StepExecution, WorkflowWithSteps } from '../../types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const getAgentModels = vi.hoisted(() => vi.fn());
vi.mock('../../lib/agentModels', () => ({ getAgentModels }));

const CATALOG = [
  {
    kind: 'claude-code',
    display_label: 'Claude Code',
    lists_models: true,
    default_model: null,
    install_command: '',
    effort_levels: ['low', 'medium', 'high'],
  },
  {
    kind: 'codex',
    display_label: 'Codex',
    lists_models: true,
    default_model: null,
    install_command: '',
    effort_levels: ['low', 'medium', 'high'],
  },
];

const MACHINE_AGENTS = [
  { kind: 'claude-code', enabled: true, available: true },
  { kind: 'codex', enabled: true, available: true },
];

function workflow(
  id: string,
  name: string,
  steps: Pick<StepConfig, 'id' | 'kind' | 'verifier'>[],
): WorkflowWithSteps {
  return {
    id,
    name,
    description: '',
    is_starter: false,
    created_at: 0,
    updated_at: 0,
    version: 1,
    version_id: `${id}-v1`,
    steps: steps.map((step) => ({ ...step, title: step.id })) as StepConfig[],
  };
}

const STARTER = workflow('wf-starter-address-review', 'Address Review Findings', [
  { id: 's-address', kind: 'agent', verifier: { instructions: '' } },
  { id: 's-finalize', kind: 'finalize' },
]);
const DEEP_FIX = workflow('wf-deep-fix', 'Deep fix', [
  { id: 's-address', kind: 'agent', verifier: { instructions: '' } },
  { id: 's-finalize', kind: 'finalize' },
]);
/** No `finalize`: it would work through the findings and publish none of it. */
const REVIEW_ONLY = workflow('wf-deep-review', 'Deep review', [{ id: 's-review', kind: 'agent' }]);
/** The bundled Experiment's shape: it publishes, but no step would run a gate. */
const UNVERIFIED_FIX = workflow('wf-experiment', 'Experiment', [
  { id: 's-try', kind: 'agent' },
  { id: 's-finalize', kind: 'finalize' },
]);

const WORKFLOWS = [STARTER, DEEP_FIX, REVIEW_ONLY, UNVERIFIED_FIX];

const REPORT = '1. The refspec guard admits a leading dash.';
const GATES = '`npm run checks` exited 1; both clippy warnings are pre-existing.';

function execution(stepId: string, artifact: string, stepIndex: number): StepExecution {
  return {
    id: `se-${stepIndex}`,
    feature_id: 'feat-1',
    step_id: stepId,
    step_index: stepIndex,
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: [`/tmp/wt/artifacts/${artifact}`],
    created_at: 0,
    updated_at: 0,
  } as unknown as StepExecution;
}

const REPORT_STEP = execution('s-review', 'code-review.md', 0);
const GATES_STEP = execution('s-validate-branch', 'branch-validation.md', 1);

const STEPS = [REPORT_STEP, GATES_STEP];

const RED_GATE_REASON = 'gate `test` exited 1: 3 failed, 41 passed';
const RED_GATES_STEP = {
  ...execution('s-validate-branch', 'branch-validation.md', 1),
  status: 'failed',
  error_message: RED_GATE_REASON,
  artifact_paths: [],
} as StepExecution;

const RUNNING_GATE = {
  ...GATES_STEP,
  status: 'running',
  artifact_paths: [],
} as StepExecution;

function pullRequest(over: Partial<PullRequestSummary> = {}): PullRequestSummary {
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
  pullRequests?: PullRequestSummary[];
  report?: string;
  reportReadFails?: boolean;
  gates?: string;
  gatesReadFails?: boolean;
  /** The reads behind the run-shape pickers, which are the ones allowed to fail
   *  without taking the action with them. */
  runShapeReadsFail?: boolean;
  /** Answers to successive `list_open_pull_requests` calls, in order; calls
   *  past the end get `pullRequests`. A pending promise here holds a re-join. */
  listings?: Promise<PullRequestSummary[]>[];
}) {
  const launches: Record<string, unknown>[] = [];
  let listingCalls = 0;
  getAgentModels.mockResolvedValue([{ value: 'gpt-5-codex', name: 'GPT-5 Codex' }]);
  vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
    if (cmd === 'workflow_list' || cmd === 'get_agent_configs') {
      if (input.runShapeReadsFail) return Promise.reject(new Error('offline'));
      return Promise.resolve(cmd === 'workflow_list' ? WORKFLOWS : MACHINE_AGENTS);
    }
    if (cmd === 'get_project_by_id') return Promise.resolve({ id: 'proj-1', remote_host: null });
    if (cmd === 'list_agents') return Promise.resolve(CATALOG);
    if (cmd === 'feature_get') {
      return Promise.resolve(
        input.fetchSpec === undefined
          ? { id: 'feat-1' }
          : { id: 'feat-1', origin: { kind: 'ref', fetch_spec: input.fetchSpec, label: 'patch-1' } },
      );
    }
    if (cmd === 'list_open_pull_requests') {
      const queued = input.listings?.[listingCalls++];
      return queued ?? Promise.resolve(input.pullRequests ?? [pullRequest()]);
    }
    if (cmd === 'get_proposed_strategy') {
      return Promise.resolve({ worktree_strategy: { default_branch: 'main' } });
    }
    if (cmd === 'artifact_body') {
      const path = artifactPath(args);
      if (path.endsWith('/code-review.md')) {
        if (input.reportReadFails) return Promise.reject(new Error('gone'));
        return Promise.resolve(input.report ?? REPORT);
      }
      if (path.endsWith('/branch-validation.md')) {
        if (input.gatesReadFails) return Promise.reject(new Error('gone'));
        return Promise.resolve(input.gates ?? GATES);
      }
      return Promise.reject(new Error(`unread artifact: ${path}`));
    }
    if (cmd === 'start_feature') {
      launches.push(typeof args === 'object' && args !== null ? { ...args } : {});
      return Promise.resolve({ id: 'feat-2', title: 'fix', status: 'running' });
    }
    return Promise.reject(new Error(`unexpected command: ${cmd}`));
  });
  return launches;
}

function artifactPath(args: unknown): string {
  if (typeof args === 'object' && args !== null && 'path' in args && typeof args.path === 'string') {
    return args.path;
  }
  return '';
}

function mount(
  onLaunch: (params: FixLaunchParams) => Promise<void>,
  steps: StepExecution[] = STEPS,
  runStatus = 'completed',
) {
  render(
    <AddressFindingsLaunch
      featureId="feat-1"
      projectId="proj-1"
      steps={steps}
      runStatus={runStatus}
      onLaunch={onLaunch}
    />,
  );
}

/** A running review, so the gate step's own status alone decides the hold. */
function launcher(
  steps: StepExecution[],
  onLaunch: (params: FixLaunchParams) => Promise<void>,
): ReactElement {
  return (
    <AddressFindingsLaunch
      featureId="feat-1"
      projectId="proj-1"
      steps={steps}
      runStatus="running"
      onLaunch={onLaunch}
    />
  );
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

function listingCalls(): number {
  return vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'list_open_pull_requests').length;
}

function optionValues(select: HTMLElement): string[] {
  return within(select)
    .getAllByRole<HTMLOptionElement>('option')
    .map((option) => option.value);
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
    expect(launched[0].diffBaseBranch).toBe('main');
    expect(String(launched[0].description)).toContain(REPORT);
  });

  // The launch is measured against the target and published to the head, so
  // copy that read the diff base would promise a pull request against main.
  it('names the head branch as the destination, not the branch it is measured against', async () => {
    backend({
      fetchSpec: 'refs/pull/412/head',
      pullRequests: [pullRequest({ target_branch: 'release/2.x' })],
    });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    });

    const hint = await screen.findByTestId('fix-hint');
    expect(hint).toHaveTextContent('against patch-1');
    expect(hint).not.toHaveTextContent('release/2.x');

    await userEvent.click(screen.getByTestId('address-findings'));
    expect(screen.getByTestId('fix-publishes-to')).toHaveTextContent('patch-1');
    expect(screen.getByRole('dialog')).not.toHaveTextContent('release/2.x');

    await userEvent.click(screen.getByTestId('address-findings-confirm'));
    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].diffBaseBranch).toBe('release/2.x');
  });

  // What separates a second pass from a rerun of the first: the fix agent is
  // told which gates went red, instead of re-deriving it from a branch it has
  // not built yet.
  it('carries the gate verdict as well as the report into the fix run', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    });

    await userEvent.click(await screen.findByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].description).toContain(REPORT);
    expect(launched[0].description).toContain(GATES);
  });

  // Every run that finished before `s-validate-branch` shipped arrives here,
  // and the body they are seeded with must not move because a second artifact
  // became possible for later ones. Asserted against the description
  // `planFixLaunch` builds from the bare report — which is the value this
  // surface passed before it composed anything — so the claim is byte
  // identity rather than the weaker "contains the report, lacks the gates",
  // which an empty gate heading appended to every run would also satisfy.
  it('seeds the fix with the report alone when the run declared no gate artifact', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    }, [REPORT_STEP]);

    await userEvent.click(await screen.findByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    const uncomposed = planFixLaunch({
      pullRequest: pullRequest(),
      findings: REPORT,
      defaultBranch: 'main',
    });
    expect(uncomposed.ok).toBe(true);
    expect(launched[0].description).toBe(uncomposed.ok && uncomposed.launch.description);
  });

  it('offers posting the report back on the request it reviewed', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    mount(async () => {});

    // The button existing in its own file is not the same as it being reachable:
    // it shipped tested and unmounted, so this asserts the mount, not the button.
    const post = await screen.findByTestId('post-review-comment');
    expect(post).toBeEnabled();
    await userEvent.click(post);
    expect(await screen.findByTestId('confirm-post-review-comment')).toBeInTheDocument();
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
    backend({ fetchSpec: 'refs/pull/412/head' });
    mount(async () => {}, [GATES_STEP]);

    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalled());
    expect(screen.queryByTestId('address-findings')).not.toBeInTheDocument();
  });

  // A declared gate artifact that cannot be opened costs the fix run its gate
  // section, not the user the action: the review report is what the fix is
  // for, and it was read. Nor does the failure reason stand in — the step
  // completed, so it has none worth quoting.
  it('seeds the report alone when the declared gate artifact cannot be read', async () => {
    backend({ fetchSpec: 'refs/pull/412/head', gatesReadFails: true });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    });

    expect(await screen.findByTestId('post-review-comment')).toBeInTheDocument();
    await userEvent.click(screen.getByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    const uncomposed = planFixLaunch({
      pullRequest: pullRequest(),
      findings: REPORT,
      defaultBranch: 'main',
    });
    expect(uncomposed.ok).toBe(true);
    expect(launched[0].description).toBe(uncomposed.ok && uncomposed.launch.description);
  });

  // A red gate ends its step before the agent that would have written
  // `branch-validation.md` runs, so the reason the engine recorded is the only
  // account of it — and it quotes the branch's own output, so it must land
  // fenced.
  it('seeds a red gate from the reason its step failed with, inside the fence', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    }, [REPORT_STEP, RED_GATES_STEP]);

    await userEvent.click(await screen.findByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    const description = launched[0].description;
    const open = description.indexOf(GATE_FENCE_OPEN.failure);
    const reason = description.indexOf(RED_GATE_REASON);
    const close = description.indexOf(GATE_FENCE_CLOSE);
    expect(open).toBeGreaterThanOrEqual(0);
    expect(reason).toBeGreaterThan(open);
    expect(close).toBeGreaterThan(reason);
    expect(description).toContain(GATE_HEADING.failure);
    expect(description).not.toContain(GATE_HEADING.report);
  });

  // `s-review` records its report minutes before `s-validate-branch` has run
  // the project's suite. A fix launched in that window would be told nothing
  // about a red branch, with nothing in the brief saying the gates were never
  // read — so the action waits for them, while the comment, which needs only
  // the report, does not.
  it('holds the fix until the gate step finishes, then seeds it with the gates', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: FixLaunchParams[] = [];
    const onLaunch = async (params: FixLaunchParams) => {
      launched.push(params);
    };
    const runningGate = {
      ...GATES_STEP,
      status: 'running',
      artifact_paths: [],
    } as StepExecution;
    const view = render(
      <AddressFindingsLaunch
        featureId="feat-1"
        projectId="proj-1"
        steps={[REPORT_STEP, runningGate]}
        runStatus="running"
        onLaunch={onLaunch}
      />,
    );

    expect(await screen.findByTestId('fix-gates-pending')).toBeInTheDocument();
    expect(screen.getByTestId('address-findings')).toBeDisabled();
    expect(screen.queryByTestId('fix-hint')).not.toBeInTheDocument();
    expect(screen.getByTestId('post-review-comment')).toBeEnabled();

    view.rerender(
      <AddressFindingsLaunch
        featureId="feat-1"
        projectId="proj-1"
        steps={STEPS}
        runStatus="running"
        onLaunch={onLaunch}
      />,
    );

    await waitFor(() => expect(screen.getByTestId('address-findings')).toBeEnabled());
    expect(screen.queryByTestId('fix-gates-pending')).not.toBeInTheDocument();
    await userEvent.click(screen.getByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].description).toContain(REPORT);
    expect(launched[0].description).toContain(GATES);
  });

  // The gate step settling re-runs the join, and until that re-join answers
  // the only evidence in hand is what was read while the gates were running:
  // the report and no gates. The listing is a provider round-trip, so that
  // window is one a user who was waiting on the pending hint will click into.
  it('keeps holding the fix after the gate step settles until the join has re-read', async () => {
    const relisting = deferred<PullRequestSummary[]>();
    backend({
      fetchSpec: 'refs/pull/412/head',
      listings: [Promise.resolve([pullRequest()]), relisting.promise],
    });
    const launched: FixLaunchParams[] = [];
    const onLaunch = async (params: FixLaunchParams) => {
      launched.push(params);
    };
    const view = render(launcher([REPORT_STEP, RUNNING_GATE], onLaunch));
    expect(await screen.findByTestId('fix-gates-pending')).toBeInTheDocument();

    view.rerender(launcher(STEPS, onLaunch));
    await waitFor(() => expect(listingCalls()).toBe(2));

    expect(screen.getByTestId('address-findings')).toBeDisabled();
    expect(screen.queryByTestId('fix-hint')).not.toBeInTheDocument();
    expect(screen.getByTestId('fix-gates-pending')).toBeInTheDocument();
    await userEvent.click(screen.getByTestId('address-findings'));
    expect(screen.queryByTestId('address-findings-confirm')).not.toBeInTheDocument();
    expect(launched).toHaveLength(0);

    relisting.resolve([pullRequest()]);
    await waitFor(() => expect(screen.getByTestId('address-findings')).toBeEnabled());
    await userEvent.click(screen.getByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].description).toContain(GATES);
  });

  it('keeps holding the fix after the gate step fails until the join has re-read', async () => {
    const relisting = deferred<PullRequestSummary[]>();
    backend({
      fetchSpec: 'refs/pull/412/head',
      listings: [Promise.resolve([pullRequest()]), relisting.promise],
    });
    const launched: FixLaunchParams[] = [];
    const onLaunch = async (params: FixLaunchParams) => {
      launched.push(params);
    };
    const view = render(launcher([REPORT_STEP, RUNNING_GATE], onLaunch));
    expect(await screen.findByTestId('fix-gates-pending')).toBeInTheDocument();

    view.rerender(launcher([REPORT_STEP, RED_GATES_STEP], onLaunch));
    await waitFor(() => expect(listingCalls()).toBe(2));

    expect(screen.getByTestId('address-findings')).toBeDisabled();
    expect(screen.queryByTestId('fix-hint')).not.toBeInTheDocument();
    await userEvent.click(screen.getByTestId('address-findings'));
    expect(launched).toHaveLength(0);

    relisting.resolve([pullRequest()]);
    await waitFor(() => expect(screen.getByTestId('address-findings')).toBeEnabled());
    await userEvent.click(screen.getByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].description).toContain(GATE_HEADING.failure);
    expect(launched[0].description).toContain(RED_GATE_REASON);
  });

  // Merged or closed while its gates ran: the re-join cannot find it, and the
  // evidence gathered before is for a launch that can no longer be seeded right.
  it('takes the action away when the re-join no longer finds the request open', async () => {
    backend({
      fetchSpec: 'refs/pull/412/head',
      listings: [Promise.resolve([pullRequest()]), Promise.resolve([])],
    });
    const onLaunch = async () => {};
    const view = render(launcher([REPORT_STEP, RUNNING_GATE], onLaunch));
    expect(await screen.findByTestId('fix-gates-pending')).toBeInTheDocument();

    view.rerender(launcher(STEPS, onLaunch));

    await waitFor(() =>
      expect(screen.queryByTestId('address-findings')).not.toBeInTheDocument(),
    );
    expect(listingCalls()).toBe(2);
  });

  // A cancel leaves the gate step's row wherever it was, and a cancelled run
  // records nothing more: the gates are not running, and never will be. What
  // is left is the report, which is what this run could launch from before
  // the hold existed.
  it.each(['running', 'interrupted'])(
    'launches from the report alone on a cancelled run whose gate step was left %s',
    async (status) => {
      backend({ fetchSpec: 'refs/pull/412/head' });
      const launched: FixLaunchParams[] = [];
      const abandonedGate = { ...GATES_STEP, status, artifact_paths: [] } as StepExecution;
      mount(
        async (params) => {
          launched.push(params);
        },
        [REPORT_STEP, abandonedGate],
        'cancelled',
      );

      expect(await screen.findByTestId('fix-hint')).toBeInTheDocument();
      expect(screen.queryByTestId('fix-gates-pending')).not.toBeInTheDocument();
      expect(screen.getByTestId('address-findings')).toBeEnabled();
      await userEvent.click(screen.getByTestId('address-findings'));
      await userEvent.click(screen.getByTestId('address-findings-confirm'));

      await waitFor(() => expect(launched).toHaveLength(1));
      const uncomposed = planFixLaunch({
        pullRequest: pullRequest(),
        findings: REPORT,
        defaultBranch: 'main',
      });
      expect(uncomposed.ok).toBe(true);
      expect(launched[0].description).toBe(uncomposed.ok && uncomposed.launch.description);
    },
  );

  // The report is the one read this surface cannot do without: there is no
  // fix to offer and no comment to post.
  it('renders nothing when the review report cannot be read', async () => {
    backend({ fetchSpec: 'refs/pull/412/head', reportReadFails: true });
    mount(async () => {});

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('artifact_body', expect.anything()),
    );
    expect(screen.queryByTestId('address-findings')).not.toBeInTheDocument();
    expect(screen.queryByTestId('post-review-comment')).not.toBeInTheDocument();
  });

  it('renders nothing for a run that was not cut from a pull request', async () => {
    backend({});
    mount(async () => {});

    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalled());
    expect(screen.queryByTestId('address-findings')).not.toBeInTheDocument();
  });

  // The run shape is what makes a second pass different from the one that just
  // finished: same findings, another harness or another pipeline. It has to
  // survive the trip into `onLaunch`, and an untouched control has to arrive as
  // `undefined` rather than as `''` — a harness named the empty string inherits
  // nothing and would pin a shape the user never chose.
  it('delivers the workflow, harness, model and effort the user chose', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    });

    await userEvent.selectOptions(await screen.findByLabelText('Workflow'), 'wf-deep-fix');
    await userEvent.selectOptions(screen.getByLabelText('Harness'), 'codex');
    await waitFor(() =>
      expect(optionValues(screen.getByLabelText('Model'))).toContain('gpt-5-codex'),
    );
    await userEvent.selectOptions(screen.getByLabelText('Model'), 'gpt-5-codex');
    await userEvent.selectOptions(screen.getByLabelText('Effort'), 'high');
    await userEvent.click(screen.getByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0]).toMatchObject({
      workflowId: 'wf-deep-fix',
      agentKind: 'codex',
      model: 'gpt-5-codex',
      effort: 'high',
    });
  });

  // The engine resolves the model apart from the harness, so an untouched model
  // under a chosen harness is the project's model id, picked for another one.
  it('holds a chosen harness until a model is chosen for it, pickers still shown', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const onLaunch = vi.fn<(params: FixLaunchParams) => Promise<void>>().mockResolvedValue();
    mount(onLaunch);

    await userEvent.selectOptions(await screen.findByLabelText('Harness'), 'codex');

    expect(screen.getByTestId('fix-model-required')).toHaveTextContent(/codex/);
    expect(screen.queryByTestId('fix-hint')).not.toBeInTheDocument();
    expect(screen.getByTestId('address-findings')).toBeDisabled();
    await userEvent.click(screen.getByTestId('address-findings'));
    expect(screen.queryByTestId('address-findings-confirm')).not.toBeInTheDocument();
    expect(onLaunch).not.toHaveBeenCalled();

    await waitFor(() =>
      expect(optionValues(screen.getByLabelText('Model'))).toContain('gpt-5-codex'),
    );
    await userEvent.selectOptions(screen.getByLabelText('Model'), 'gpt-5-codex');

    expect(screen.queryByTestId('fix-model-required')).not.toBeInTheDocument();
    await userEvent.click(screen.getByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));
    await waitFor(() => expect(onLaunch).toHaveBeenCalledTimes(1));
    expect(onLaunch.mock.calls[0][0]).toMatchObject({ agentKind: 'codex', model: 'gpt-5-codex' });
  });

  it('keeps the fix held when the chosen harness offers no model', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    getAgentModels.mockResolvedValue([]);
    const onLaunch = vi.fn<(params: FixLaunchParams) => Promise<void>>().mockResolvedValue();
    mount(onLaunch);

    await userEvent.selectOptions(await screen.findByLabelText('Harness'), 'codex');
    await waitFor(() => expect(optionValues(screen.getByLabelText('Model'))).toEqual(['']));

    expect(screen.getByTestId('fix-model-required')).toHaveTextContent(/Project default/);
    expect(screen.getByTestId('address-findings')).toBeDisabled();
    expect(onLaunch).not.toHaveBeenCalled();
  });

  it('inherits every part of the run shape the user left alone', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    });

    await userEvent.click(await screen.findByTestId('address-findings'));
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].workflowId).toBe(FIX_STARTER_WORKFLOW_ID);
    expect(launched[0].agentKind).toBeUndefined();
    expect(launched[0].model).toBeUndefined();
    expect(launched[0].effort).toBeUndefined();
  });

  it('names the workflow the run will use at the moment the user confirms', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    mount(async () => {});

    await userEvent.click(await screen.findByTestId('address-findings'));
    expect(screen.getByTestId('fix-chosen-workflow')).toHaveTextContent('Address Review Findings');

    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await userEvent.selectOptions(screen.getByLabelText('Workflow'), 'wf-deep-fix');
    await userEvent.click(screen.getByTestId('address-findings'));
    expect(screen.getByTestId('fix-chosen-workflow')).toHaveTextContent('Deep fix');
  });

  // The review chooser's filter is the wrong way round here: a fix run's job is
  // to publish, and the starter it defaults to carries a `finalize` step. A
  // workflow that publishes with no verifier step would publish ungated, under
  // copy promising every gate still applies.
  it('offers only workflows that both publish and verify the fix', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    mount(async () => {});

    expect(optionValues(await screen.findByLabelText('Workflow'))).toEqual([
      '',
      'wf-starter-address-review',
      'wf-deep-fix',
    ]);
  });

  // The join above is wrapped so that a failure takes the action away. These
  // reads are not: they decide what the pickers hold, and no picker is the run
  // this surface launched before it had any.
  it('still launches on the defaults when the picker reads fail', async () => {
    backend({ fetchSpec: 'refs/pull/412/head', runShapeReadsFail: true });
    const launched: FixLaunchParams[] = [];
    mount(async (params) => {
      launched.push(params);
    });

    await userEvent.click(await screen.findByTestId('address-findings'));
    expect(screen.queryByLabelText('Workflow')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Harness')).not.toBeInTheDocument();
    await userEvent.click(screen.getByTestId('address-findings-confirm'));

    await waitFor(() => expect(launched).toHaveLength(1));
    expect(launched[0].workflowId).toBe(FIX_STARTER_WORKFLOW_ID);
  });

  // Gate output is not findings. A review that wrote nothing has nothing to act
  // on, however loud the suite was, and a run seeded with the gate section
  // alone is an agent asked to fix a red build nobody reported.
  it('refuses a review that produced no report, however much the gates said', async () => {
    backend({ fetchSpec: 'refs/pull/412/head', report: '   \n\t' });
    mount(async () => {});

    expect(await screen.findByTestId('fix-refused')).toHaveTextContent('no report');
    expect(screen.getByTestId('address-findings')).toBeDisabled();
  });

  // A fork's fix run is cut from the fork's head, so its preparation and gate
  // commands run the fork author's code exactly as the review's did.
  it("names a fork's code running here before the fix starts", async () => {
    backend({
      fetchSpec: 'refs/pull/412/head',
      pullRequests: [pullRequest({ from_fork: true, head_repo_push: false })],
    });
    mount(async () => {});

    await userEvent.click(await screen.findByTestId('address-findings'));
    expect(within(screen.getByRole('dialog')).getByTestId('fix-fork-notice')).toHaveTextContent(
      FORK_EXPOSURE,
    );
  });

  it('names no fork exposure for a same-repo pull request', async () => {
    backend({ fetchSpec: 'refs/pull/412/head' });
    mount(async () => {});

    await userEvent.click(await screen.findByTestId('address-findings'));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.queryByTestId('fix-fork-notice')).not.toBeInTheDocument();
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
