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

// The step under gate review, in the shape the executor writes: a gate step
// carries its predecessor's `artifact_paths` copied verbatim (`steps/gate/
// mod.rs`), never a path of its own. A fixture that invents one hides whether
// the default lands on a row the picker actually renders.
const GATE_STEP = step({
  id: 'se-gate',
  step_id: 's-gate-review',
  step_index: 2,
  step_kind: 'gate',
  artifact_paths: ['artifacts/implementation-spec.md'],
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
  it('defaults to the artifact the gate inherited once data loads, with the picker rendered above the viewer', async () => {
    mount();

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent(
        'artifacts/implementation-spec.md',
      );
    });

    // Picker rows for both predecessors are present above the viewer panel.
    expect(screen.getByText('s-research')).toBeInTheDocument();
    expect(screen.getByText('s-spec')).toBeInTheDocument();

    const picker = screen.getByText('s-research').closest('div');
    const viewer = screen.getByTestId('artifact-viewer-stub');
    expect(picker).not.toBeNull();
    expect(picker!.compareDocumentPosition(viewer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  // The default now names the step that produced the inherited path, so its row
  // is the highlighted one — a path-only match would light up every step that
  // declares it, the gate included.
  it('highlights the producing step\'s row for the inherited default', async () => {
    mount();

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent(
        'artifacts/implementation-spec.md',
      );
    });

    const rows = screen
      .getAllByText('implementation-spec.md')
      .map((el) => el.closest('button'))
      .filter((row) => row?.classList.contains('border-violet-500/30'));
    expect(rows).toHaveLength(1);
    expect(screen.queryByText('s-gate-review')).not.toBeInTheDocument();
  });

  // The stranding `9ce030b` fixed for `s-tickets`, one step upstream: an agent
  // step whose first declared path is a source edit hands the gate an inherited
  // path the picker folds away, so nothing highlights and the reviewer's first
  // click elsewhere is one-way.
  it('opens the nearest predecessor\'s listable row when the inherited path has none', async () => {
    const implement = step({
      id: 'se-implement',
      step_id: 's-implement',
      step_index: 1,
      artifact_paths: ['src/lib/auth.ts', 'artifacts/implementation-report.md'],
    });
    const gate = step({
      id: 'se-gate',
      step_id: 's-gate-review',
      step_index: 2,
      step_kind: 'gate',
      artifact_paths: ['src/lib/auth.ts', 'artifacts/implementation-report.md'],
    });
    mount({ gateStep: gate, allSteps: [RESEARCH, implement, gate] });

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent(
        'artifacts/implementation-report.md',
      );
    });

    const row = screen.getByText('implementation-report.md').closest('button');
    expect(row).toHaveClass('border-violet-500/30');
  });

  it('changes the artifactPath the viewer receives when a different predecessor row is selected', async () => {
    mount();

    await waitFor(() => {
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent(
        'artifacts/implementation-spec.md',
      );
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
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent(
        'artifacts/implementation-spec.md',
      );
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
      expect(screen.getByTestId('artifact-viewer-stub')).toHaveTextContent(
        'artifacts/implementation-spec.md',
      );
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
