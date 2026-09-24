// What this surface has to get right is the difference between *chosen* and
// *untouched*. A chosen harness has to survive the trip into `onReview`, and an
// untouched one has to arrive as `undefined` rather than as `''` — a harness
// named the empty string, which inherits nothing and would pin a run shape the
// user never asked for. Both directions are asserted on the argument the row
// would hand `useLaunchRun`, because that is the only place the difference is
// still visible.

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { PullRequestLaunch, REVIEW_FORK_NOTICE } from './PullRequestLaunch';
import type { ReviewRunInputs } from './ReviewRunOptions';
import type { AgentAvailability } from '../../lib/featureDetail';
import type { PullRequestSummary } from '../../lib/pullRequests';
import { REVIEW_STARTER_WORKFLOW_ID, type ReviewLaunchParams } from '../../lib/reviewLaunch';
import type { StepConfig, WorkflowWithSteps } from '../../types';

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

const MACHINE_AGENTS: AgentAvailability[] = [
  { kind: 'claude-code', enabled: true, available: true },
  { kind: 'codex', enabled: true, available: true },
];

function workflow(
  id: string,
  steps: Pick<StepConfig, 'id' | 'kind' | 'verifier'>[],
): WorkflowWithSteps {
  return {
    id,
    name: id,
    description: '',
    is_starter: false,
    created_at: 0,
    updated_at: 0,
    version: 1,
    version_id: `${id}-v1`,
    steps: steps.map((step) => ({ ...step, title: step.id })),
  };
}

const REVIEW_ONLY = workflow('wf-deep-review', [
  { id: 's-review', kind: 'agent' },
  { id: 's-validate', kind: 'agent', verifier: { instructions: '' } },
]);
const UNGATED = workflow('wf-prose-review', [{ id: 's-review', kind: 'agent' }]);
const PUBLISHES = workflow('wf-pipeline', [
  { id: 's-implement', kind: 'agent' },
  { id: 's-finalize', kind: 'finalize' },
]);

const RUN_INPUTS: ReviewRunInputs = {
  workflows: [REVIEW_ONLY, UNGATED, PUBLISHES],
  machineAgents: MACHINE_AGENTS,
  machineId: 'local',
};

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

function mount(over: { pullRequest?: PullRequestSummary; runOptions?: ReviewRunInputs } = {}) {
  const onReview = vi.fn<(params: ReviewLaunchParams) => Promise<void>>().mockResolvedValue();
  render(
    <PullRequestLaunch
      pullRequest={over.pullRequest ?? summary()}
      onReview={onReview}
      agentKind="claude-code"
      runOptions={'runOptions' in over ? over.runOptions : RUN_INPUTS}
    />,
  );
  return { onReview };
}

async function openOptions() {
  await userEvent.click(screen.getByTestId('review-options-toggle'));
}

function optionValues(select: HTMLElement): string[] {
  return within(select)
    .getAllByRole<HTMLOptionElement>('option')
    .map((option) => option.value);
}

beforeEach(() => {
  getAgentModels.mockResolvedValue([{ value: 'gpt-5-codex', name: 'GPT-5 Codex' }]);
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd === 'list_agents'
      ? Promise.resolve(CATALOG)
      : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
});

describe('PullRequestLaunch', () => {
  it('delivers the workflow, harness, model and effort the user chose', async () => {
    const { onReview } = mount();

    await openOptions();
    await userEvent.selectOptions(screen.getByLabelText('Workflow'), 'wf-deep-review');
    await userEvent.selectOptions(screen.getByLabelText('Harness'), 'codex');
    await waitFor(() =>
      expect(optionValues(screen.getByLabelText('Model'))).toContain('gpt-5-codex'),
    );
    await userEvent.selectOptions(screen.getByLabelText('Model'), 'gpt-5-codex');
    await userEvent.selectOptions(screen.getByLabelText('Effort'), 'high');
    await userEvent.click(screen.getByTestId('review-this-pr'));

    expect(onReview).toHaveBeenCalledTimes(1);
    expect(onReview.mock.calls[0][0]).toMatchObject({
      workflowId: 'wf-deep-review',
      agentKind: 'codex',
      model: 'gpt-5-codex',
      effort: 'high',
    });
  });

  it('inherits every part of the run shape the user left alone', async () => {
    const { onReview } = mount();

    await userEvent.click(screen.getByTestId('review-this-pr'));

    const launch = onReview.mock.calls[0][0];
    expect(launch.workflowId).toBe(REVIEW_STARTER_WORKFLOW_ID);
    expect(launch.agentKind).toBeUndefined();
    expect(launch.model).toBeUndefined();
    expect(launch.effort).toBeUndefined();
  });

  it('does not offer a workflow that can publish, or that runs no gate', async () => {
    mount();

    await openOptions();

    expect(optionValues(screen.getByLabelText('Workflow'))).toEqual(['', 'wf-deep-review']);
  });

  it('offers no run-shape controls at all to a parent that fetched none', async () => {
    const { onReview } = mount({ runOptions: undefined });

    await userEvent.click(screen.getByTestId('review-options-toggle'));

    expect(screen.queryByLabelText('Workflow')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Harness')).not.toBeInTheDocument();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview.mock.calls[0][0].workflowId).toBe(REVIEW_STARTER_WORKFLOW_ID);
  });

  // The engine resolves the model apart from the harness, so an untouched model
  // under a chosen harness is the project's model id, picked for another one.
  it('holds a chosen harness until a model is chosen for it', async () => {
    const { onReview } = mount();

    await openOptions();
    await userEvent.selectOptions(screen.getByLabelText('Harness'), 'codex');

    expect(screen.getByTestId('review-model-required')).toHaveTextContent(/codex/);
    expect(screen.queryByTestId('review-hint')).not.toBeInTheDocument();
    expect(screen.getByTestId('review-this-pr')).toBeDisabled();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).not.toHaveBeenCalled();

    await waitFor(() =>
      expect(optionValues(screen.getByLabelText('Model'))).toContain('gpt-5-codex'),
    );
    await userEvent.selectOptions(screen.getByLabelText('Model'), 'gpt-5-codex');

    expect(screen.queryByTestId('review-model-required')).not.toBeInTheDocument();
    expect(screen.getByTestId('review-this-pr')).toBeEnabled();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview.mock.calls[0][0]).toMatchObject({ agentKind: 'codex', model: 'gpt-5-codex' });
  });

  it('keeps the launch held when the chosen harness offers no model', async () => {
    getAgentModels.mockResolvedValue([]);
    const { onReview } = mount();

    await openOptions();
    await userEvent.selectOptions(screen.getByLabelText('Harness'), 'codex');
    await waitFor(() => expect(optionValues(screen.getByLabelText('Model'))).toEqual(['']));

    expect(screen.getByTestId('review-model-required')).toHaveTextContent(/Project default/);
    expect(screen.getByTestId('review-this-pr')).toBeDisabled();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).not.toHaveBeenCalled();
  });

  it('still refuses a pull request with no fetchable head, whatever the picker holds', async () => {
    const { onReview } = mount({ pullRequest: summary({ head_fetch_spec: '' }) });

    await openOptions();
    await userEvent.selectOptions(screen.getByLabelText('Harness'), 'codex');

    expect(screen.getByTestId('review-refused')).toBeInTheDocument();
    expect(screen.queryByTestId('review-hint')).not.toBeInTheDocument();
    expect(screen.queryByTestId('review-model-required')).not.toBeInTheDocument();
    expect(screen.getByTestId('review-this-pr')).toBeDisabled();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).not.toHaveBeenCalled();
  });
});

describe('PullRequestLaunch — a fork pull request', () => {
  const FORK = summary({ from_fork: true, head_repo_path: 'mallory/app' });

  it('runs nothing until the user acknowledges what reviewing a fork executes', async () => {
    const { onReview } = mount({ pullRequest: FORK });

    expect(screen.getByTestId('review-fork-notice')).toHaveTextContent(REVIEW_FORK_NOTICE);
    const consent = screen.getByTestId('review-fork-consent');
    expect(consent).not.toBeChecked();
    expect(screen.getByTestId('review-this-pr')).toBeDisabled();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).not.toHaveBeenCalled();

    await userEvent.click(consent);
    await userEvent.click(screen.getByTestId('review-this-pr'));

    expect(onReview).toHaveBeenCalledTimes(1);
  });

  it('spends one acknowledgement on one launch', async () => {
    const { onReview } = mount({ pullRequest: FORK });

    await userEvent.click(screen.getByTestId('review-fork-consent'));
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).toHaveBeenCalledTimes(1);

    await waitFor(() => expect(screen.getByTestId('review-fork-consent')).not.toBeChecked());
    expect(screen.getByTestId('review-this-pr')).toBeDisabled();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).toHaveBeenCalledTimes(1);

    await userEvent.click(screen.getByTestId('review-fork-consent'));
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).toHaveBeenCalledTimes(2);
  });

  it('keeps the one-click launch for a pull request from this repository', async () => {
    const { onReview } = mount();

    expect(screen.queryByTestId('review-fork-notice')).not.toBeInTheDocument();
    expect(screen.queryByTestId('review-fork-consent')).not.toBeInTheDocument();
    await userEvent.click(screen.getByTestId('review-this-pr'));

    expect(onReview).toHaveBeenCalledTimes(1);
  });

  it('does not carry an acknowledgement over to a different pull request', async () => {
    const onReview = vi.fn<(params: ReviewLaunchParams) => Promise<void>>().mockResolvedValue();
    const view = (pullRequest: PullRequestSummary) => (
      <PullRequestLaunch
        pullRequest={pullRequest}
        onReview={onReview}
        agentKind="claude-code"
        runOptions={RUN_INPUTS}
      />
    );
    const { rerender } = render(view(FORK));
    await userEvent.click(screen.getByTestId('review-fork-consent'));
    expect(screen.getByTestId('review-fork-consent')).toBeChecked();

    rerender(
      view(
        summary({
          number: 413,
          web_url: 'https://github.com/acme/app/pull/413',
          from_fork: true,
          head_repo_path: 'eve/app',
          head_fetch_spec: 'refs/pull/413/head',
        }),
      ),
    );

    expect(screen.getByTestId('review-fork-consent')).not.toBeChecked();
    expect(screen.getByTestId('review-this-pr')).toBeDisabled();
    await userEvent.click(screen.getByTestId('review-this-pr'));
    expect(onReview).not.toHaveBeenCalled();
  });
});
