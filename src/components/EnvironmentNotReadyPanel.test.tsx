// HB7 — a terminal environment failure must not look like a failed feature.
//
// The claims these rest on:
//
//   1. The **remediation is the primary content**, not a line buried inside a
//      monospace error dump. It is the entire payload of this failure class,
//      and the failure exists precisely because the machine — not the change —
//      is wrong.
//
//   2. A gate that was already unrunnable at the base says so. The run stopped
//      at the baseline node with no implement budget spent (HB9), which is a
//      different event from a fault that appeared mid-run, and the two are
//      byte-identical in the message.
//
//   3. A failure with no remediation says so rather than rendering a blank
//      "Do this" panel — the classifier is not obliged to know a fix, and a
//      silent empty box reads as a rendering bug.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { EnvironmentNotReadyPanel } from './EnvironmentNotReadyPanel';
import type { EnvironmentFailure } from '../lib/harnessVerdict';

const FAILURE: EnvironmentFailure = {
  reason: 'The build needs the gdk-3.0 development headers, which are not installed.',
  remediation: "install libgtk-3-dev, then check with:\n  bash -l -i -c 'pkg-config gdk-3.0'",
  command: 'cargo test',
  machine: 'runner-01',
  reproduce: '  ssh runner-01\n  cd /wt && cargo test',
};

describe('EnvironmentNotReadyPanel', () => {
  it('renders the remediation as primary content, not as an error string', () => {
    render(<EnvironmentNotReadyPanel failure={FAILURE} atBase={false} />);

    const remediation = screen.getByTestId('environment-remediation');
    expect(remediation).toHaveTextContent('install libgtk-3-dev');
    // Prose, not the monospace dump the raw `error_message` used to be — the
    // action is meant to be read, not decoded.
    expect(remediation).toHaveClass('font-sans');
    expect(screen.getByText('Do this')).toBeInTheDocument();

    // And it does not read as the feature's defect.
    expect(
      screen.getByText(/Environment not ready — the machine, not the feature/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Editing the code cannot fix it/)).toBeInTheDocument();
  });

  it('keeps the reason, command, machine and reproduce line as supporting evidence', () => {
    render(<EnvironmentNotReadyPanel failure={FAILURE} atBase={false} />);
    expect(screen.getByText(FAILURE.reason)).toBeInTheDocument();
    expect(screen.getByText('cargo test')).toBeInTheDocument();
    expect(screen.getByText('runner-01')).toBeInTheDocument();
    expect(screen.getByText(/ssh runner-01/)).toBeInTheDocument();
  });

  it('says when the gate was already unrunnable at the base', () => {
    render(<EnvironmentNotReadyPanel failure={FAILURE} atBase />);
    expect(
      screen.getByText(/already failing at the base commit because this machine cannot run it/),
    ).toBeInTheDocument();
    expect(screen.getByText(/before any implementation budget was spent/)).toBeInTheDocument();
  });

  it('says so when the classifier suggested no remediation', () => {
    render(<EnvironmentNotReadyPanel failure={{ ...FAILURE, remediation: '' }} atBase={false} />);
    expect(screen.queryByTestId('environment-remediation')).not.toBeInTheDocument();
    expect(screen.getByTestId('environment-no-remediation')).toHaveTextContent(
      /No remediation was suggested/,
    );
  });
});
