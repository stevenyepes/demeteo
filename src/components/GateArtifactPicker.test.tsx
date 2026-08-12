// GateArtifactPicker — lets a gate reviewer pick any predecessor artifact,
// not only the immediate predecessor's (spec §3/§6).

import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { GateArtifactPicker } from './GateArtifactPicker';
import type { StepExecution } from '../types';

function step(overrides: Partial<StepExecution> & Pick<StepExecution, 'step_id' | 'step_index'>): StepExecution {
  return {
    id: `se-${overrides.step_id}`,
    feature_id: 'f-1',
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: [],
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

const BASELINE = step({
  step_id: 's-baseline-harness',
  step_index: 0,
  step_kind: 'command',
  artifact_paths: [],
});

const RESEARCH = step({
  step_id: 's-research',
  step_index: 1,
  artifact_paths: ['artifacts/research-report.md'],
});

const SPEC = step({
  step_id: 's-spec',
  step_index: 2,
  artifact_paths: ['artifacts/implementation-spec.md'],
});

// The path the standard pipeline actually declares for this step, and its only
// one — an earlier fixture used `artifacts/tickets.md`, a file no workflow
// writes, which is precisely what hid the markdown-only fold that dropped this
// step (and with it the ticket list) out of the picker entirely.
const TICKETS = step({
  step_id: 's-tickets',
  step_index: 3,
  artifact_paths: ['artifacts/task-list.json'],
});

const STEPS = [BASELINE, RESEARCH, SPEC, TICKETS];

describe('GateArtifactPicker', () => {
  it('renders every predecessor step with a listable artifact as a selectable row, grouped by step', () => {
    render(
      <GateArtifactPicker
        steps={STEPS}
        gateStepIndex={4}
        selectedArtifactPath={null}
        selectedStepId={null}
        onSelectArtifact={vi.fn()}
      />,
    );

    expect(screen.getByText('s-research')).toBeInTheDocument();
    expect(screen.getByText('research-report.md')).toBeInTheDocument();
    expect(screen.getByText('s-spec')).toBeInTheDocument();
    expect(screen.getByText('implementation-spec.md')).toBeInTheDocument();
    expect(screen.getByText('s-tickets')).toBeInTheDocument();
    expect(screen.getByText('task-list.json')).toBeInTheDocument();
  });

  it('lists the ticket list of a step that declares nothing else, in the shipped pipeline shape', () => {
    render(
      <GateArtifactPicker
        steps={[BASELINE, RESEARCH, SPEC, TICKETS]}
        gateStepIndex={4}
        selectedArtifactPath="artifacts/implementation-spec.md"
        selectedStepId="s-spec"
        onSelectArtifact={vi.fn()}
      />,
    );

    // The reviewer has clicked away to the spec. Without a row for the ticket
    // list there is no way back to it short of closing the modal.
    const row = screen.getByText('task-list.json').closest('button');
    expect(row).not.toBeNull();
    expect(screen.queryByText(/no reviewable artifacts/i)).not.toBeInTheDocument();
  });

  it('omits a step with nothing listable entirely, not as an empty group', () => {
    render(
      <GateArtifactPicker
        steps={STEPS}
        gateStepIndex={4}
        selectedArtifactPath={null}
        selectedStepId={null}
        onSelectArtifact={vi.fn()}
      />,
    );

    expect(screen.queryByText('s-baseline-harness')).not.toBeInTheDocument();
  });

  it('never includes a step at or after the gate step_index', () => {
    render(
      <GateArtifactPicker
        steps={STEPS}
        gateStepIndex={2}
        selectedArtifactPath={null}
        selectedStepId={null}
        onSelectArtifact={vi.fn()}
      />,
    );

    expect(screen.getByText('s-research')).toBeInTheDocument();
    expect(screen.queryByText('s-spec')).not.toBeInTheDocument();
    expect(screen.queryByText('s-tickets')).not.toBeInTheDocument();
  });

  it('invokes onSelectArtifact with (path, step_id) when a non-immediate-predecessor row is clicked', () => {
    const onSelectArtifact = vi.fn();
    render(
      <GateArtifactPicker
        steps={STEPS}
        gateStepIndex={4}
        selectedArtifactPath={null}
        selectedStepId={null}
        onSelectArtifact={onSelectArtifact}
      />,
    );

    // Immediate predecessor is s-tickets (step_index 3); click s-research
    // (step_index 1) instead.
    fireEvent.click(screen.getByText('research-report.md'));

    expect(onSelectArtifact).toHaveBeenCalledWith('artifacts/research-report.md', 's-research');
  });

  it('marks the row matching selectedArtifactPath as selected', () => {
    render(
      <GateArtifactPicker
        steps={STEPS}
        gateStepIndex={4}
        selectedArtifactPath="artifacts/implementation-spec.md"
        selectedStepId="s-spec"
        onSelectArtifact={vi.fn()}
      />,
    );

    const row = screen.getByText('implementation-spec.md').closest('button');
    expect(row).toHaveClass('border-violet-500/30');
  });

  // Two steps can declare the same path — a rework cycle rewrites the artifact
  // its first cycle wrote — and a path-only match then paints both rows as the
  // current selection, so the reviewer cannot tell which one they are reading.
  it('selects only the row of the step the path was opened from', () => {
    const REWORK = step({
      step_id: 's-tickets-rework',
      step_index: 4,
      artifact_paths: ['artifacts/task-list.json'],
    });

    render(
      <GateArtifactPicker
        steps={[TICKETS, REWORK]}
        gateStepIndex={5}
        selectedArtifactPath="artifacts/task-list.json"
        selectedStepId="s-tickets-rework"
        onSelectArtifact={vi.fn()}
      />,
    );

    const rows = screen.getAllByText('task-list.json').map((el) => el.closest('button'));
    expect(rows).toHaveLength(2);
    expect(rows.filter((row) => row?.classList.contains('border-violet-500/30'))).toHaveLength(1);
  });

  // A gate copies its predecessor's artifact_paths verbatim so its own card can
  // show them, so listing gates repeats the producing step's rows under a step
  // that wrote nothing. Every shipped workflow has two gates.
  it('omits an earlier gate rather than repeating its predecessor artifacts', () => {
    const FIRST_GATE = step({
      step_id: 's-gate-review',
      step_index: 4,
      step_kind: 'gate',
      artifact_paths: ['artifacts/task-list.json'],
    });

    render(
      <GateArtifactPicker
        steps={[...STEPS, FIRST_GATE]}
        gateStepIndex={5}
        selectedArtifactPath={null}
        selectedStepId={null}
        onSelectArtifact={vi.fn()}
      />,
    );

    expect(screen.queryByText('s-gate-review')).not.toBeInTheDocument();
    expect(screen.getAllByText('task-list.json')).toHaveLength(1);
  });

  it('renders an empty state, not a crash, when there are zero reviewable predecessors', () => {
    render(
      <GateArtifactPicker
        steps={[BASELINE]}
        gateStepIndex={1}
        selectedArtifactPath={null}
        selectedStepId={null}
        onSelectArtifact={vi.fn()}
      />,
    );

    expect(screen.getByText(/no reviewable artifacts/i)).toBeInTheDocument();
  });

  it('groups artifacts under a status chip per step', () => {
    render(
      <GateArtifactPicker
        steps={STEPS}
        gateStepIndex={4}
        selectedArtifactPath={null}
        selectedStepId={null}
        onSelectArtifact={vi.fn()}
      />,
    );

    const chips = screen.getAllByTestId('chip');
    expect(chips).toHaveLength(3);
    within(chips[0]).getByText(/completed/i);
  });
});
