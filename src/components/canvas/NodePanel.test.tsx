/**
 * `NodePanel` (task P2.3): the node drill-down panel. These prove the two
 * read-only tabs surface the right Phase-1 data — the Overview tab's per-attempt
 * table from `step_attempts_list` (the row the timeline overwrites on retry) and
 * the failure class, and the Output tab's harness/verifier output + artifact
 * list — so a failure's root cause is reachable from whichever run surface is
 * showing, without opening another one.
 *
 * `ArtifactViewer` is mocked out: it only mounts when an artifact is selected
 * (these tests assert the chooser, not the body) and pulls Monaco otherwise.
 */
import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../ArtifactViewer', () => ({
  ArtifactViewer: ({ artifactPath }: { artifactPath: string | null }) => (
    <div data-testid="artifact-viewer">{artifactPath}</div>
  ),
}));

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { NodePanel } from './NodePanel';
import type { NodeConfigV2, NodeRunStatus } from './types';
import type { AgentStreamStore } from '../FeatureDetail/useAgentStream';
import type { HarnessOverrides } from '../FeatureDetail/useHarnessOverrides';
import type { StepAssignment } from '../FeatureDetail/useStepAssignment';
import { MANUAL_SYNC_STEP_ID } from '../../lib/featureSync';
import type { HarnessBaseline, StepAttempt, StepExecution } from '../../types';

const node = (over: Partial<NodeConfigV2> = {}): NodeConfigV2 => ({
  id: 'implement',
  type: 'agent',
  title: 'Implement Feature',
  ...over,
});

const attempt = (over: Partial<StepAttempt>): StepAttempt => ({
  step_execution_id: 'se-1',
  attempt_no: 1,
  status: 'failed',
  started_at: 0,
  ...over,
});

const step = (over: Partial<StepExecution> = {}): StepExecution => ({
  id: 'se-1',
  feature_id: 'f1',
  step_id: 'implement',
  step_index: 0,
  step_kind: 'agent',
  status: 'failed',
  artifact_paths: [],
  created_at: 0,
  updated_at: 1,
  ...over,
});

/** Byte-for-byte the shape `build_environment_message` composes, since
 *  `parseEnvironmentFailure` reports nothing for anything else and a panel that
 *  never renders would pass an "and not the raw block" assertion by itself. */
const ENVIRONMENT_MESSAGE =
  'Environment not ready — this failure is not something editing the code can fix.\n\n' +
  'cargo is not on the PATH of the login shell.\n' +
  'Remediation: install rustup on the machine.\n\n' +
  'Failing command: cargo test\n' +
  'Machine: local\n' +
  'Reproduce:\n  cd /wt && cargo test\n';

const BASELINE: HarnessBaseline = {
  base_sha: 'abc123',
  harnesses: [
    {
      name: 'unit',
      command: 'cargo test',
      exit_ok: false,
      measured_at: 1,
      producer: 'node',
      environment: { reason: 'cargo missing', remediation: 'install rustup' },
    },
  ],
};

const overrides = (over: Partial<HarnessOverrides> = {}): HarnessOverrides => ({
  machineAgents: [],
  availableModels: [],
  selectedModel: '',
  setSelectedModel: vi.fn(),
  isLoadingModels: false,
  availableAgents: ['opencode'],
  selectedAgent: '',
  selectedEffort: '',
  setSelectedEffort: vi.fn(),
  seededEffort: '',
  featureAgentKind: 'opencode',
  inheritedAgentKind: 'opencode',
  retryEffortLevels: ['low', 'high'],
  onAgentChange: vi.fn(),
  adoptFeatureModel: vi.fn(),
  probeForFeature: vi.fn(),
  ...over,
});

const assignment = (over: Partial<StepAssignment> = {}): StepAssignment => ({
  dirty: false,
  pinned: true,
  applying: false,
  error: null,
  clearError: vi.fn(),
  apply: vi.fn(async () => {}),
  reset: vi.fn(async () => {}),
  ...over,
});

afterEach(() => {
  cleanup();
  invoke.mockReset();
});

describe('NodePanel — Overview', () => {
  it('renders the per-attempt table from step_attempts_list', async () => {
    invoke.mockResolvedValue([
      attempt({ attempt_no: 1, status: 'failed', error_class: 'agent_failure', cost_usd: 0.12, wall_clock_ms: 4200, applied_rule: 'agent_failure.in_place' }),
      attempt({ attempt_no: 2, status: 'completed', cost_usd: 0.08, wall_clock_ms: 3100 }),
    ]);
    const run: NodeRunStatus = { status: 'completed', costUsd: 0.2, wallClockSecs: 7, stepExecutionId: 'se-1' };

    render(
      <NodePanel featureId="f1" node={node()} run={run} step={step({ status: 'completed' })} onClose={() => {}} />,
    );

    await waitFor(() => expect(screen.getByText('agent_failure.in_place')).toBeInTheDocument());
    // Both attempts present, keyed by their number.
    expect(screen.getByText('Agent failure')).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('step_attempts_list', { executionId: 'se-1' });
  });

  it('shows the failure-class chip in the header when the node failed', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', errorClass: 'verdict', stepExecutionId: 'se-1' };
    render(<NodePanel featureId="f1" node={node()} run={run} step={step()} onClose={() => {}} />);
    // "Verdict" appears as the class label chip.
    expect(screen.getByText('Verdict')).toBeInTheDocument();
    await waitFor(() => expect(invoke).toHaveBeenCalled()); // let the attempts fetch settle
  });

  it('hints "not started" and skips the fetch for a node with no execution', () => {
    render(<NodePanel featureId="f1" node={node()} run={null} step={null} onClose={() => {}} />);
    expect(screen.getByText(/hasn't started yet/i)).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalled();
  });

  // The unified run-event feed used to render at the bottom of this tab. It is
  // run-level, not node-level, and now lives in the run's own `ActivityPanel`
  // (UI_REDESIGN_PLAN §1 D) — `ActivityPanel.test.tsx` carries what this
  // asserted. Pinned here so the tab does not re-grow a run-level section.
  it('keeps the Overview tab node-scoped — no run-level activity log', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'completed' })}
        onClose={() => {}}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.queryByText('Run activity')).not.toBeInTheDocument();
  });
});

describe('NodePanel — layout', () => {
  it('states no width of its own and takes the one its host gives it', () => {
    // The pane's floor and ceiling belong to the `SplitPane` divider's clamp
    // (`splitPaneGeometry.ts`), which the user drags. A width spelled on the
    // panel as well would fight that clamp, so the panel carries none.
    const { container } = render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={null}
        step={null}
        onClose={() => {}}
        className="h-full"
      />,
    );
    const root = container.firstElementChild;
    expect(root).not.toBeNull();
    const cls = root!.className;
    expect(cls).toContain('h-full');
    expect(cls).not.toMatch(/(^|\s)(w-|min-w-|max-w-|basis-)/);
  });
});

describe('NodePanel — Output', () => {
  it('shows harness/verifier output and the artifact chooser', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ error_message: '2 tests failed: auth_spec.rs', artifact_paths: ['artifacts/report.md'] })}
        onClose={() => {}}
      />,
    );

    await waitFor(() => expect(invoke).toHaveBeenCalled()); // settle the Overview fetch first
    fireEvent.click(screen.getByText('Output'));
    expect(screen.getByText(/2 tests failed: auth_spec.rs/)).toBeInTheDocument();
    // Artifact appears in the chooser by basename; its body only mounts on click.
    expect(screen.getByText('report.md')).toBeInTheDocument();
    expect(screen.queryByTestId('artifact-viewer')).not.toBeInTheDocument();

    fireEvent.click(screen.getByText('report.md'));
    expect(screen.getByTestId('artifact-viewer')).toHaveTextContent('artifacts/report.md');
  });

  it('delegates the artifact click to onOpenArtifact instead of previewing inline', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    const onOpenArtifact = vi.fn();
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'completed', artifact_paths: ['artifacts/report.md'] })}
        onClose={() => {}}
        onOpenArtifact={onOpenArtifact}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Output'));
    fireEvent.click(screen.getByText('report.md'));

    expect(onOpenArtifact).toHaveBeenCalledWith('artifacts/report.md');
    // The host owns the preview — the panel mounts no viewer of its own.
    expect(screen.queryByTestId('artifact-viewer')).not.toBeInTheDocument();
  });

  it('renders an empty state when the node produced no output', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(<NodePanel featureId="f1" node={node()} run={run} step={step({ status: 'completed' })} onClose={() => {}} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled()); // settle the Overview fetch first
    fireEvent.click(screen.getByText('Output'));
    expect(screen.getByText(/No output produced/i)).toBeInTheDocument();
  });
});

describe('NodePanel — Output: artifacts an agent step declared', () => {
  const openOutput = async (over: Partial<StepExecution>) => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'completed', ...over })}
        onClose={() => {}}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Output'));
  };

  it('lists the document and folds the files it touched into a count', async () => {
    await openOutput({
      artifact_paths: ['artifacts/report.md', 'src/lib/auth.ts', 'src/lib/auth.test.ts'],
    });
    expect(screen.getByText('report.md')).toBeInTheDocument();
    expect(screen.queryByText('auth.ts')).not.toBeInTheDocument();
    expect(screen.getByText(/2 files changed/)).toBeInTheDocument();
  });

  it('still counts as output when every declared path was folded away', async () => {
    // The empty state and the fold rule are derived from the same list, so a
    // step that produced only source edits must not read as having produced
    // nothing at all.
    await openOutput({ artifact_paths: ['src/lib/auth.ts'] });
    expect(screen.queryByText(/No output produced/i)).not.toBeInTheDocument();
    expect(screen.getByText(/1 file changed/)).toBeInTheDocument();
  });

  it('lists every path a non-agent step declared', async () => {
    await openOutput({
      step_kind: 'gate',
      artifact_paths: ['artifacts/verdict.json', 'changes.patch'],
    });
    expect(screen.getByText('verdict.json')).toBeInTheDocument();
    expect(screen.getByText('changes.patch')).toBeInTheDocument();
    expect(screen.queryByText(/files? changed/)).not.toBeInTheDocument();
  });
});

describe('NodePanel — Output: environment failures', () => {
  const openOutput = async (over: Partial<StepExecution>, baseline?: HarnessBaseline) => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed', ...over })}
        onClose={() => {}}
        harnessBaseline={baseline}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Output'));
  };

  it('presents the remediation instead of the raw failure text', async () => {
    await openOutput({ error_message: ENVIRONMENT_MESSAGE });
    expect(screen.getByTestId('environment-not-ready')).toBeInTheDocument();
    expect(screen.getByTestId('environment-remediation')).toHaveTextContent(
      'install rustup on the machine.',
    );
    expect(screen.queryByText(/Verifier \/ harness output/i)).not.toBeInTheDocument();
  });

  it('keeps the raw block for a failure the feature actually caused', async () => {
    await openOutput({ error_message: '2 tests failed: auth_spec.rs' });
    expect(screen.queryByTestId('environment-not-ready')).not.toBeInTheDocument();
    expect(screen.getByText(/2 tests failed: auth_spec.rs/)).toBeInTheDocument();
  });

  it('says the run stopped at the baseline when the baseline already knew', async () => {
    await openOutput({ error_message: ENVIRONMENT_MESSAGE }, BASELINE);
    expect(screen.getByTestId('environment-not-ready')).toHaveTextContent(
      /already failing at the base commit/i,
    );
  });

  it('reads as a run-time fault with no baseline in hand', async () => {
    // The canvas mounts this panel with no baseline at all, and the two
    // wordings are different claims about what was spent.
    await openOutput({ error_message: ENVIRONMENT_MESSAGE });
    expect(screen.getByTestId('environment-not-ready')).toHaveTextContent(
      /never produced a result here/i,
    );
  });
});

describe('NodePanel — Overview: sequence task list (P2.5)', () => {
  // A command-aware mock: the sequence Overview fires two reads.
  const routed = (seq: unknown) =>
    invoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === 'sequence_tasks_list' ? seq : []),
    );

  it('renders the landed prefix distinctly from pending tasks', async () => {
    routed({
      planned: true,
      tasks: [
        { id: 't1', title: 'Scaffold module', status: 'landed', landed: true, cost_usd: 0.4 },
        { id: 't2', title: 'Wire the handler', status: 'running', landed: false, cost_usd: 0.1 },
        { id: 't3', title: 'Add tests', status: 'pending', landed: false },
      ],
    });
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node({ id: 'implement', type: 'sequence', title: 'Implement' })}
        run={run}
        step={step({ step_kind: 'sequence', status: 'running' })}
        onClose={() => {}}
      />,
    );

    await waitFor(() => expect(screen.getByText('Scaffold module')).toBeInTheDocument());
    // Landed count summary + landed chip.
    expect(screen.getByText('1/3 landed')).toBeInTheDocument();
    expect(screen.getByText('Landed')).toBeInTheDocument();
    expect(screen.getByText('Wire the handler')).toBeInTheDocument();
    expect(screen.getByText('Add tests')).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('sequence_tasks_list', {
      featureId: 'f1',
      nodeId: 'implement',
      executionId: 'se-1',
    });
  });

  it('groups a reworked node by decomposition cycle', async () => {
    // A downstream verdict sent the run back to the step that produces the
    // task list, which emitted a delta. Both lists are on the branch, so both
    // render — showing only the delta would present two tickets as the whole
    // feature.
    routed({
      planned: true,
      tasks: [
        { id: 't1', title: 'Scaffold module', status: 'landed', landed: true, cycle: 0, prior_cycle: true },
        { id: 't2', title: 'Wire the handler', status: 'landed', landed: true, cycle: 0, prior_cycle: true },
        { id: 'fix-1', title: 'Debounce the search', status: 'running', landed: false, cycle: 1, prior_cycle: false },
      ],
    });
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node({ id: 'implement', type: 'sequence', title: 'Implement' })}
        run={run}
        step={step({ step_kind: 'sequence', status: 'running' })}
        onClose={() => {}}
      />,
    );

    await waitFor(() => expect(screen.getByText('Scaffold module')).toBeInTheDocument());
    expect(screen.getByText('Original decomposition')).toBeInTheDocument();
    expect(screen.getByText('Rework 1')).toBeInTheDocument();
    expect(screen.getByText('2 tickets')).toBeInTheDocument();
    expect(screen.getByText('1 ticket')).toBeInTheDocument();
    expect(screen.getByText('Debounce the search')).toBeInTheDocument();
  });

  it('shows no cycle headers on a node that has only ever planned once', async () => {
    // The common case. A "Cycle 0" header there names a distinction that
    // does not exist yet.
    routed({
      planned: true,
      tasks: [
        { id: 't1', title: 'Scaffold module', status: 'landed', landed: true, cycle: 0, prior_cycle: false },
        { id: 't2', title: 'Wire the handler', status: 'pending', landed: false, cycle: 0, prior_cycle: false },
      ],
    });
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node({ id: 'implement', type: 'sequence', title: 'Implement' })}
        run={run}
        step={step({ step_kind: 'sequence', status: 'running' })}
        onClose={() => {}}
      />,
    );

    await waitFor(() => expect(screen.getByText('Scaffold module')).toBeInTheDocument());
    expect(screen.queryByText('Original decomposition')).not.toBeInTheDocument();
    expect(screen.queryByText(/^Rework /)).not.toBeInTheDocument();
  });

  it('stays silent for a sequence node that has not planned yet', async () => {
    routed({ planned: false, tasks: [] });
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node({ type: 'sequence', title: 'Implement' })}
        run={run}
        step={step({ step_kind: 'sequence' })}
        onClose={() => {}}
      />,
    );
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('sequence_tasks_list', expect.anything()),
    );
    expect(screen.queryByText('Task list')).not.toBeInTheDocument();
  });

  it('does not fetch a task list for a non-sequence node', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(<NodePanel featureId="f1" node={node()} run={run} step={step()} onClose={() => {}} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(invoke).not.toHaveBeenCalledWith('sequence_tasks_list', expect.anything());
  });
});

describe('NodePanel — Live', () => {
  /** Answers for one execution id only: a store that returns the same text for
   *  any id would pass even if the tab subscribed to the wrong step. */
  const storeFor = (
    stepExecutionId: string,
    text: string,
    truncated = false,
  ): AgentStreamStore => ({
    subscribe: () => () => {},
    read: (id) => (id === stepExecutionId ? text : ''),
    isTruncated: (id) => id === stepExecutionId && truncated,
  });

  it('shows the agent-stream buffer while running', () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'running' })}
        onClose={() => {}}
        streamStore={storeFor('se-1', 'thinking about the change…')}
        isStreaming
      />,
    );
    fireEvent.click(screen.getByText('Live'));
    expect(screen.getByText(/thinking about the change/)).toBeInTheDocument();
  });

  it('says the buffer is a tail once the cap has dropped anything', () => {
    // Without it the last N KB of a long turn reads as everything the agent
    // said, and this tab is the only place the stream is now mounted.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'running' })}
        onClose={() => {}}
        streamStore={storeFor('se-1', 'later output', true)}
        isStreaming
      />,
    );
    fireEvent.click(screen.getByText('Live'));
    expect(screen.getByText(/Earlier output dropped/i)).toBeInTheDocument();
  });

  it('stays silent about truncation while the whole turn is still buffered', () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'running' })}
        onClose={() => {}}
        streamStore={storeFor('se-1', 'the whole turn')}
        isStreaming
      />,
    );
    fireEvent.click(screen.getByText('Live'));
    expect(screen.queryByText(/Earlier output dropped/i)).not.toBeInTheDocument();
  });

  it('hints when the node is not running and has no buffer', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(<NodePanel featureId="f1" node={node()} run={run} step={step({ status: 'completed' })} onClose={() => {}} />);
    fireEvent.click(screen.getByText('Live'));
    expect(screen.getByText(/No live output/i)).toBeInTheDocument();
    await waitFor(() => expect(invoke).toHaveBeenCalled()); // settle the attempts fetch
  });
});

describe('NodePanel — Actions', () => {
  it('disables Retry with an ancestor explanation when blocked', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    const onRetry = vi.fn();
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed' })}
        onClose={() => {}}
        onRetry={onRetry}
        blockedBy={{ step_id: 'research', status: 'running' }}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    const retry = screen.getByRole('button', { name: 'Retry' });
    expect(retry).toBeDisabled();
    fireEvent.click(retry);
    expect(onRetry).not.toHaveBeenCalled();
    // The guard reason is spelled out, not just a disabled button.
    expect(screen.getByText(/Ancestor "research" is still running/)).toBeInTheDocument();
  });

  it('fires Retry and Replay when unblocked', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    const onRetry = vi.fn();
    const onReplay = vi.fn();
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed' })}
        onClose={() => {}}
        onRetry={onRetry}
        onReplay={onReplay}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    fireEvent.click(screen.getByRole('button', { name: 'Replay…' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(onReplay).toHaveBeenCalledTimes(1);
  });

  it('offers Decide on an awaiting gate node', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'awaiting_gate', stepExecutionId: 'se-g' };
    const onDecideGate = vi.fn();
    render(
      <NodePanel
        featureId="f1"
        node={node({ id: 'gate-ship', type: 'gate', title: 'Ship Gate' })}
        run={run}
        step={step({ id: 'se-g', step_id: 'gate-ship', step_kind: 'gate', status: 'awaiting_gate' })}
        onClose={() => {}}
        onDecideGate={onDecideGate}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    fireEvent.click(screen.getByRole('button', { name: 'Decide' }));
    expect(onDecideGate).toHaveBeenCalledTimes(1);
  });

  // A `sequence` node that parked itself for a human writes the same
  // `awaiting_gate` a gate step does, and `gate_decide` answers both. When
  // this was gated on `node.type === 'gate'` the panel offered no action at
  // all — `isFailed` does not cover `awaiting_gate` either — so the only way
  // to answer was the transient toast.
  it('offers Decide on a parked non-gate node', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'awaiting_gate', stepExecutionId: 'se-i' };
    const onDecideGate = vi.fn();
    const onRetry = vi.fn();
    render(
      <NodePanel
        featureId="f1"
        node={node({ id: 'implement', type: 'sequence', title: 'Implement Tickets' })}
        run={run}
        step={step({
          id: 'se-i',
          step_id: 'implement',
          step_kind: 'sequence',
          status: 'awaiting_gate',
        })}
        onClose={() => {}}
        onDecideGate={onDecideGate}
        onRetry={onRetry}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    fireEvent.click(screen.getByRole('button', { name: 'Decide' }));
    expect(onDecideGate).toHaveBeenCalledTimes(1);
    // Retry takes the `replay_steps_from` path, which rewinds the row and
    // arms a second driver against a run the parked one is still holding.
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('offers the assignment picker alongside Retry on a failed node', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed' })}
        onClose={() => {}}
        onRetry={() => {}}
        overrides={overrides()}
        assignment={assignment()}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.getByText('Assignment')).toBeInTheDocument();
    expect(screen.getByLabelText('Harness')).toBeInTheDocument();
    expect(screen.getByLabelText('Effort')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
  });

  it('offers no assignment on a node that spawns no agent', async () => {
    // A gate reads no harness, so an Apply there would report a pin nothing
    // consults. The picker's leftover edit from another node must not hold
    // this node's Replay shut either.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'pending', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node({ id: 'gate-ship', type: 'gate', title: 'Ship Gate' })}
        run={run}
        step={step({ status: 'pending' })}
        onClose={() => {}}
        onReplay={() => {}}
        overrides={overrides({ selectedAgent: 'claude-code' })}
        assignment={assignment({ dirty: true })}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    expect(screen.queryByText('Assignment')).toBeNull();
    expect(screen.queryByLabelText('Harness')).toBeNull();
    expect(screen.getByRole('button', { name: 'Replay…' })).toBeEnabled();
  });

  it('holds Retry and Replay shut while the picker has an edit nobody applied', async () => {
    // Both rerun paths submit null for agent/model/effort, so they re-run on
    // the stored pin. The picker sits one row above them and used to feed
    // them, so the mismatch has to stop the press — a sentence in the rows'
    // body copy is read past by anyone who has already decided to click.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    const onRetry = vi.fn();
    const onReplay = vi.fn();
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed' })}
        onClose={() => {}}
        onRetry={onRetry}
        onReplay={onReplay}
        overrides={overrides({ selectedAgent: 'claude-code' })}
        assignment={assignment({ dirty: true })}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    const retryBtn = screen.getByRole('button', { name: 'Retry' });
    const replayBtn = screen.getByRole('button', { name: 'Replay…' });
    expect(retryBtn).toBeDisabled();
    expect(replayBtn).toBeDisabled();
    fireEvent.click(retryBtn);
    fireEvent.click(replayBtn);
    expect(onRetry).not.toHaveBeenCalled();
    expect(onReplay).not.toHaveBeenCalled();

    // Named on screen, not only in the button's hover text.
    expect(screen.getByText(/unapplied edits.*press Apply/i)).toBeInTheDocument();

    // And the reason stays out of the rows' own descriptions.
    const rows = screen.getAllByTestId('action-row');
    const retry = rows.find((r) => r.textContent?.includes('Retry node'))!;
    const replay = rows.find((r) => r.textContent?.includes('Replay from node'))!;
    for (const row of [retry, replay]) {
      expect(row.textContent).not.toMatch(/with the assignment above/i);
      expect(row.textContent).toMatch(/pinned assignment/i);
      expect(row.textContent).not.toMatch(/unapplied edits/i);
    }
  });

  it('keeps both reasons on screen when an ancestor blocks a dirty picker too', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed' })}
        onClose={() => {}}
        onRetry={() => {}}
        onReplay={() => {}}
        blockedBy={{ step_id: 'research', status: 'running' }}
        overrides={overrides({ selectedAgent: 'claude-code' })}
        assignment={assignment({ dirty: true })}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    expect(screen.getByText(/Ancestor "research" is still running/)).toBeInTheDocument();
    expect(screen.getByText(/unapplied edits.*press Apply/i)).toBeInTheDocument();
  });

  it('leaves the rerun rows untouched while the picker matches the pin', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    const onRetry = vi.fn();
    const onReplay = vi.fn();
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed' })}
        onClose={() => {}}
        onRetry={onRetry}
        onReplay={onReplay}
        overrides={overrides()}
        assignment={assignment({ dirty: false })}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    const retry = screen
      .getAllByTestId('action-row')
      .find((r) => r.textContent?.includes('Retry node'))!;
    expect(retry.textContent).toMatch(/pinned assignment/i);
    expect(screen.queryByText(/unapplied edits/i)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Replay…' })).toBeEnabled();

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalled();
  });

  it('leads the Actions tab on a queued node that has other controls too', async () => {
    // The `isFailed` branch is the one this used to sit in, so a node with a
    // rerun row available is where a regression back into it would still read
    // as passing: Assignment has to stand above the row, not instead of it.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'pending', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'pending' })}
        onClose={() => {}}
        onReplay={() => {}}
        overrides={overrides()}
        assignment={assignment()}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));

    const heading = screen.getByText('Assignment');
    expect(screen.getByLabelText('Harness')).toBeInTheDocument();
    expect(screen.queryByText(/Read-only/i)).not.toBeInTheDocument();
    const replay = screen.getByRole('button', { name: 'Replay…' });
    expect(heading.compareDocumentPosition(replay) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('reaches the Actions tab for a queued node whose only control is Assignment', async () => {
    // The node this feature exists for: nothing has run, so no retry, replay,
    // stop or gate decision applies, and the old gate hid the whole tab.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'pending', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'pending' })}
        onClose={() => {}}
        overrides={overrides()}
        assignment={assignment()}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.queryByText(/No actions available/i)).not.toBeInTheDocument();
    expect(screen.getByText('Assignment')).toBeInTheDocument();
    expect(screen.getByLabelText('Harness')).toBeInTheDocument();
  });

  it('shows a running node its assignment read-only, not as a picker', async () => {
    // The agent is already up: a select here would take a choice the backend
    // (`domain::step_assignment::assignment_refusal`) refuses.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'running' })}
        onClose={() => {}}
        onStop={() => {}}
        overrides={overrides()}
        assignment={assignment()}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.getByText('Assignment')).toBeInTheDocument();
    expect(screen.getByText(/This node is running/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
    expect(screen.queryByLabelText('Harness')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Apply' })).not.toBeInTheDocument();
  });

  it("shows a running node what it spawned with, not the picker's unapplied edit", async () => {
    // The panel's read-only trio comes off the node's launch evidence, which
    // `useRunGraph` joins onto the run status — so it agrees with the
    // `AssignmentChips` on the same screen instead of echoing a picker the
    // user never applied.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = {
      status: 'running',
      stepExecutionId: 'se-1',
      agentKind: 'opencode',
      model: 'gpt-5.6',
      effort: 'high',
    };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'running' })}
        onClose={() => {}}
        overrides={overrides({ selectedAgent: 'claude-code', selectedModel: 'sonnet', selectedEffort: 'max' })}
        assignment={assignment()}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.getByTitle('Harness')).toHaveTextContent(/^opencode$/);
    expect(screen.getByTitle('Model')).toHaveTextContent(/^gpt-5\.6$/);
    expect(screen.getByTitle('Effort')).toHaveTextContent(/^High$/);
    expect(screen.queryByText(/claude code/i)).toBeNull();
    expect(screen.queryByText('sonnet')).toBeNull();
  });

  it('makes no spawn claim for a running node with no launch evidence', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'running', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'running' })}
        onClose={() => {}}
        overrides={overrides({ selectedAgent: 'claude-code' })}
        assignment={assignment()}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.queryByText(/was spawned with/)).toBeNull();
    expect(screen.queryByTitle('Harness')).toBeNull();
    expect(screen.getByText(/no launch evidence/i)).toBeInTheDocument();
    expect(screen.getByText(/cannot be re-pointed mid-flight/)).toBeInTheDocument();
  });

  it('offers no assignment on an out-of-band sync node', async () => {
    // A manual sync is in no graph, so there is no node to pin anything to —
    // the backend refuses `RunAction::Assign` for it.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'pending', stepExecutionId: 'se-s' };
    render(
      <NodePanel
        featureId="f1"
        node={node({ id: MANUAL_SYNC_STEP_ID, title: 'Sync with master' })}
        run={run}
        step={step({ id: 'se-s', step_id: MANUAL_SYNC_STEP_ID, status: 'pending' })}
        onClose={() => {}}
        overrides={overrides()}
        assignment={assignment()}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.queryByText('Assignment')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Harness')).not.toBeInTheDocument();
    expect(screen.getByText(/No actions available/i)).toBeInTheDocument();
  });

  it('keeps the retry a caller passed no assignment for', async () => {
    // The canvas mounts this panel with neither picker nor writer in hand.
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'failed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'failed' })}
        onClose={() => {}}
        onRetry={() => {}}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.queryByText('Assignment')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Harness')).not.toBeInTheDocument();
  });

  it('shows an empty state when no actions apply', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(<NodePanel featureId="f1" node={node()} run={run} step={step({ status: 'completed' })} onClose={() => {}} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.getByText(/No actions available/i)).toBeInTheDocument();
  });
});
