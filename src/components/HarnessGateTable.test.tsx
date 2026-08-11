// HB7 — the per-gate table that makes decision 44's subtraction auditable.
//
// The claims these rest on:
//
//   1. An excluded pre-existing failure is **named**, with the commit it was
//      identically red at. A subtraction the user cannot audit will not be
//      trusted the first time it is wrong, and that is the entire value of
//      validate no longer blaming the feature for failures it did not cause.
//
//   2. A baseline-vs-now difference renders per gate. One gate green before and
//      red now, beside one red before and excluded now, must read as two
//      different events — the attribution `&&`-chained commands used to destroy.
//
//   3. A feature with no baseline renders without inventing one. **Absent must
//      not read as green**: that inversion is the thing decision 44 exists to
//      prevent, and it is the one failure direction that is not survivable.

import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { HarnessGateTable } from './HarnessGateTable';
import { readHarnessEvidence } from '../lib/harnessVerdict';
import type { HarnessBaseline, StepExecution } from '../types';

const BASE_SHA = 'abcdef0123456789abcdef';

/** `lint` was already red at the base; `unit` was green there. */
const BASELINE: HarnessBaseline = {
  base_sha: BASE_SHA,
  harnesses: [
    {
      name: 'lint',
      command: 'npm run lint',
      exit_ok: false,
      fingerprint: 'fp-lint',
      measured_at: 1_700_000_000,
      producer: 'node',
    },
    {
      name: 'unit',
      command: 'cargo test',
      exit_ok: true,
      fingerprint: '',
      measured_at: 1_700_000_000,
      producer: 'node',
    },
  ],
};

/** What the engine persists when `unit` goes red and `lint` is subtracted. */
const VERDICT_MESSAGE =
  "'unit' — command 'cargo test' exited with failure:\n" +
  'test tokens::budget ... FAILED\n' +
  "\n\nAlso red, but NOT part of this verdict: 'lint'. That gate was already failing " +
  'identically before this feature started, so it is excluded — do not try to fix it.';

function failedStep(errorMessage: string | null): StepExecution {
  return {
    id: 'se-1',
    feature_id: 'f-1',
    step_id: 's-validate',
    step_index: 0,
    step_kind: 'agent',
    status: 'failed',
    artifact_paths: [],
    error_message: errorMessage,
    created_at: 0,
    updated_at: 0,
  };
}

function rowFor(name: string): HTMLElement {
  const row = document.querySelector<HTMLElement>(`[data-gate-row="${name}"]`);
  if (!row) throw new Error(`no gate block for '${name}'`);
  return row;
}

describe('HarnessGateTable', () => {
  it('names the excluded pre-existing failure and the commit it was red at', () => {
    render(
      <HarnessGateTable
        baseline={BASELINE}
        evidence={readHarnessEvidence([failedStep(VERDICT_MESSAGE)])}
      />,
    );

    const lint = within(rowFor('lint'));
    expect(lint.getByText('excluded — pre-existing')).toBeInTheDocument();
    // Naming the gate is not enough on its own: the audit sentence has to say
    // *what it was compared against*, or the user cannot tell a correct
    // subtraction from a bug.
    expect(
      lint.getByText(/'lint' failed with the identical output at base commit abcdef012345/),
    ).toBeInTheDocument();
    expect(
      lint.getByText(/measured at the head of this run, before any work started/),
    ).toBeInTheDocument();
  });

  it('renders the baseline-vs-now difference per gate', () => {
    render(
      <HarnessGateTable
        baseline={BASELINE}
        evidence={readHarnessEvidence([failedStep(VERDICT_MESSAGE)])}
      />,
    );

    // Green before, red now — the feature broke it.
    const unit = within(rowFor('unit'));
    expect(unit.getByText('passed')).toBeInTheDocument();
    expect(unit.getByText('failed — this feature')).toBeInTheDocument();

    // Red before, red now, identically — not the feature's.
    const lint = within(rowFor('lint'));
    expect(lint.getByText('already failing')).toBeInTheDocument();
    expect(lint.queryByText('failed — this feature')).not.toBeInTheDocument();
  });

  it('says a gate the baseline calls unrunnable could not run, with the reason', () => {
    render(
      <HarnessGateTable
        baseline={{
          base_sha: BASE_SHA,
          harnesses: [
            {
              name: 'unit',
              command: 'cargo test',
              exit_ok: false,
              fingerprint: 'fp',
              environment: {
                reason: 'The build needs gdk-3.0, which is not installed on this machine.',
                remediation: 'install libgtk-3-dev',
              },
              measured_at: 1,
              producer: 'node',
            },
          ],
        }}
        evidence={null}
      />,
    );

    const unit = within(rowFor('unit'));
    expect(unit.getByText('could not run here')).toBeInTheDocument();
    expect(
      unit.getByText('The build needs gdk-3.0, which is not installed on this machine.'),
    ).toBeInTheDocument();
    // A gate that reached no verdict is not a pre-existing defect to subtract.
    expect(unit.queryByText(/excluded from this step's verdict/i)).not.toBeInTheDocument();
  });

  it('renders a run with no baseline without inventing one', () => {
    render(
      <HarnessGateTable baseline={null} evidence={readHarnessEvidence([failedStep(VERDICT_MESSAGE)])} />,
    );

    expect(screen.getByTestId('harness-no-baseline')).toBeInTheDocument();
    const unit = within(rowFor('unit'));
    expect(unit.getByText('not measured')).toBeInTheDocument();
    // Absent is not green. Nothing anywhere may claim this gate passed before.
    expect(screen.queryByText('passed')).not.toBeInTheDocument();
    // …and no exclusion may be claimed against a commit nothing measured.
    expect(screen.queryByText(/base commit [0-9a-f]/)).not.toBeInTheDocument();
  });

  // Width is the layout owner's decision, not the card's: a cap here would
  // re-narrow the panel on a wide viewport no matter what the page decided.
  it('fills the width it is given', () => {
    render(
      <HarnessGateTable
        baseline={BASELINE}
        evidence={readHarnessEvidence([failedStep(VERDICT_MESSAGE)])}
      />,
    );

    const className = screen.getByTestId('harness-gate-table').className;
    expect(className).toContain('w-full');
    expect(className).not.toContain('max-w-');
  });

  it('renders nothing when there is neither a baseline nor a reported gate', () => {
    const { container } = render(<HarnessGateTable baseline={null} evidence={null} />);
    expect(container).toBeEmptyDOMElement();
  });
});
