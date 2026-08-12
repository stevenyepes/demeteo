// GateView — the gate-decision modal wires GateArtifactPicker +
// useArtifactSelection so a reviewer can preview any predecessor's
// artifact, not only the one the gate step itself carries (spec §6).

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

const getStepExecution = vi.fn();
const listStepsForRun = vi.fn();
const decideGate = vi.fn();

vi.mock('../lib/features', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/features')>();
  return {
    ...actual,
    getStepExecution: (...args: unknown[]) => getStepExecution(...args),
    listStepsForRun: (...args: unknown[]) => listStepsForRun(...args),
    decideGate: (...args: unknown[]) => decideGate(...args),
  };
});

vi.mock('../lib/remoteRuns', () => ({
  remoteRunForFeature: vi.fn().mockResolvedValue(null),
  decideRemoteGate: vi.fn(),
}));

vi.mock('./ArtifactViewer', () => ({
  ArtifactViewer: ({ artifactPath, contentVersion }: { artifactPath: string | null; contentVersion?: string }) => (
    <div data-testid="artifact-viewer-stub" data-content-version={contentVersion ?? ''}>
      {artifactPath}
    </div>
  ),
}));

import { GateView } from './GateView';
import type { StepExecution } from '../types';

function step(overrides: Partial<StepExecution> & Pick<StepExecution, 'id' | 'step_id' | 'step_index'>): StepExecution {
  return {
    feature_id: 'f-1',
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: [],
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

const RESEARCH = step({
  id: 'se-research',
  step_id: 's-research',
  step_index: 0,
  artifact_paths: ['artifacts/research-report.md'],
});

const SPEC = step({
  id: 'se-spec',
  step_id: 's-spec',
  step_index: 1,
  artifact_paths: ['artifacts/implementation-spec.md'],
});

// The step under gate review — its own artifact is today's default, and it
// sits strictly after both predecessors above.
const GATE_STEP = step({
  id: 'se-gate',
  step_id: 's-gate-review',
  step_index: 2,
  artifact_paths: ['artifacts/gate-review.md'],
});

function mount(overrides: { gateStep?: StepExecution; allSteps?: StepExecution[] } = {}) {
  const gateStep = overrides.gateStep ?? GATE_STEP;
  const allSteps = overrides.allSteps ?? [RESEARCH, SPEC, gateStep];
  getStepExecution.mockResolvedValue(gateStep);
  listStepsForRun.mockResolvedValue(allSteps);
  return render(
    <GateView stepExecutionId={gateStep.id} onDecisionSubmitted={vi.fn()} onClose={vi.fn()} />,
  );
}

describe('GateView artifact picker wiring', () => {
  it('defaults to the reviewed step\'s own artifact once data loads, with the picker rendered above the viewer', async () => {
    mount();

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent('artifacts/gate-review.md');
    });

    // Picker rows for both predecessors are present above the viewer panel.
    expect(screen.getByText('s-research')).toBeInTheDocument();
    expect(screen.getByText('s-spec')).toBeInTheDocument();

    const picker = screen.getByText('s-research').closest('div');
    const viewer = screen.getByTestId('artifact-viewer-stub');
    expect(picker).not.toBeNull();
    expect(picker!.compareDocumentPosition(viewer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('changes the artifactPath the viewer receives when a different predecessor row is selected', async () => {
    mount();

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent('artifacts/gate-review.md');
    });

    await userEvent.click(screen.getByText('research-report.md'));

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent('artifacts/research-report.md');
    });
  });

  it('still renders the blocking-predecessor banner when findActivePredecessor reports a blocker', async () => {
    const runningPredecessor = step({
      id: 'se-spec',
      step_id: 's-spec',
      step_index: 1,
      status: 'running',
      artifact_paths: ['artifacts/implementation-spec.md'],
    });
    mount({ allSteps: [RESEARCH, runningPredecessor, GATE_STEP] });

    await waitFor(() => {
      expect(screen.getByTestId('gate-blocked-banner')).toBeInTheDocument();
    });

    // The picker and viewer still render normally alongside the banner. The
    // default-artifact effect commits in a render pass separate from the one
    // that surfaces the banner, so it needs its own wait rather than a bare
    // assertion right after the banner appears.
    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent('artifacts/gate-review.md');
    });
    expect(screen.getByRole('button', { name: /approve step/i })).toBeDisabled();
  });

  it('prompts the reviewer to pick a row when the gate step carries no artifact of its own', async () => {
    // A redirect reset writes back an empty `artifact_paths` (see
    // `redirect_reset.rs`), so there is no default to select — but the
    // predecessors' artifacts are all listed in the picker above.
    const gateWithNoArtifact = step({
      id: 'se-gate',
      step_id: 's-gate-review',
      step_index: 2,
      artifact_paths: [],
    });
    mount({ gateStep: gateWithNoArtifact, allSteps: [RESEARCH, SPEC, gateWithNoArtifact] });

    await waitFor(() => {
      expect(screen.getByText('s-research')).toBeInTheDocument();
    });

    expect(screen.getByText(/select an artifact above/i)).toBeInTheDocument();
    expect(screen.queryByText(/no artifact outputs saved/i)).not.toBeInTheDocument();
  });

  it('still says nothing was saved when there is genuinely nothing to review', async () => {
    const lonelyGate = step({
      id: 'se-gate',
      step_id: 's-gate-review',
      step_index: 1,
      artifact_paths: [],
    });
    const baseline = step({
      id: 'se-baseline',
      step_id: 's-baseline-harness',
      step_index: 0,
      step_kind: 'command',
      artifact_paths: [],
    });
    mount({ gateStep: lonelyGate, allSteps: [baseline, lonelyGate] });

    await waitFor(() => {
      expect(screen.getByText(/no artifact outputs saved/i)).toBeInTheDocument();
    });
  });

  it('keeps the redirect feedback textarea working alongside the picker state', async () => {
    mount();

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent('artifacts/gate-review.md');
    });

    await userEvent.click(screen.getByRole('button', { name: /redirect \/ loop/i }));
    const textarea = await screen.findByPlaceholderText(/instruct the agent/i);
    await userEvent.type(textarea, 'please fix the typo');

    expect(textarea).toHaveValue('please fix the typo');
    // Selecting a different artifact still works while redirecting.
    await userEvent.click(screen.getByText('research-report.md'));
    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent('artifacts/research-report.md');
    });
  });
});
