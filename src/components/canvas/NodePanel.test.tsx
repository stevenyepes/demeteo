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

  it('renders an empty state when the node produced no output', async () => {
    invoke.mockResolvedValue([]);
    const run: NodeRunStatus = { status: 'completed', stepExecutionId: 'se-1' };
    render(<NodePanel featureId="f1" node={node()} run={run} step={step({ status: 'completed' })} onClose={() => {}} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled()); // settle the Overview fetch first
    fireEvent.click(screen.getByText('Output'));
    expect(screen.getByText(/No output produced/i)).toBeInTheDocument();
  });
});
