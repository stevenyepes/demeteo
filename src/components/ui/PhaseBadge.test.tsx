// Unit tests for the PhaseBadge presentational primitive (spec §5).
//
// Pins the phase → { label, colour } mapping mirrored from the inline
// phase ternary in `src/components/TerminalSurface.tsx`, plus the
// `data-phase` attribute consumers key off.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { PhaseBadge, type TerminalPhase } from './PhaseBadge';

const CASES: Array<{ phase: TerminalPhase; label: string }> = [
  { phase: 'connecting', label: 'Connecting' },
  { phase: 'running', label: 'Running' },
  { phase: 'disconnected', label: 'Disconnected' },
  { phase: 'closed', label: 'Closed' },
  { phase: 'error', label: 'Error' },
];

describe('PhaseBadge', () => {
  for (const { phase, label } of CASES) {
    it(`renders the ${phase} phase with its label and data-phase`, () => {
      render(<PhaseBadge phase={phase} />);

      const badge = screen.getByTestId('phase-badge');
      expect(badge).toHaveAttribute('data-phase', phase);
      expect(badge).toHaveTextContent(label);
    });
  }

  it('colours the disconnected badge amber', () => {
    render(<PhaseBadge phase="disconnected" />);

    expect(screen.getByTestId('phase-badge')).toHaveClass('text-amber-400');
  });

  it('colours the error badge ruby', () => {
    render(<PhaseBadge phase="error" />);

    expect(screen.getByTestId('phase-badge')).toHaveClass('text-ruby-400');
  });

  it('merges a caller-supplied className', () => {
    render(<PhaseBadge phase="running" className="ml-1" />);

    expect(screen.getByTestId('phase-badge')).toHaveClass('ml-1');
  });
});
