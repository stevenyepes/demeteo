/**
 * `NodePanel` (task P2.3): the node drill-down panel. These prove the two
 * read-only tabs surface the right Phase-1 data — the Overview tab's per-attempt
 * table from `step_attempts_list` (the row the timeline overwrites on retry) and
 * the failure class, and the Output tab's harness/verifier output + artifact
 * list — so a failure's root cause is reachable without leaving the graph.
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
import type { StepAttempt, StepExecution } from '../../types';

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

  it('renders the unified run-event feed (P2.6) when events are passed', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(
      <NodePanel
        featureId="f1"
        node={node()}
        run={run}
        step={step({ status: 'completed' })}
        onClose={() => {}}
        runEvents={[
          { offset: 1, run_id: 'f1', kind: 'pr_opened', payload_json: JSON.stringify('https://ex/pr/1'), created_at: 0 },
        ]}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.getByText('Run activity')).toBeInTheDocument();
    expect(screen.getByText('PR opened')).toBeInTheDocument();
  });
});

describe('NodePanel — layout', () => {
  it('clamps its width', () => {
    // A flat `w-[62%]` is a ~2100px panel at 4K and squeezes the canvas to 38%
    // in a side-by-side window. The basis needs a floor and a ceiling.
    const { container } = render(
      <NodePanel featureId="f1" node={node()} run={null} step={null} onClose={() => {}} />,
    );
    const root = container.firstElementChild;
    expect(root).not.toBeNull();
    const cls = root!.className;
    expect(cls).toContain('min-w-[20rem]');
    expect(cls).toContain('max-w-[38rem]');
    expect(cls).not.toContain('w-[62%]');
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
        liveStream={'thinking about the change…'}
        isStreaming
      />,
    );
    fireEvent.click(screen.getByText('Live'));
    expect(screen.getByText(/thinking about the change/)).toBeInTheDocument();
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

  it('shows an empty state when no actions apply', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(<NodePanel featureId="f1" node={node()} run={run} step={step({ status: 'completed' })} onClose={() => {}} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    fireEvent.click(screen.getByText('Actions'));
    expect(screen.getByText(/No actions available/i)).toBeInTheDocument();
  });
});
