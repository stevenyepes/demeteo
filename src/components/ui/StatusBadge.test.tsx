// Unit tests for the StatusBadge dot (UI redesign plan §5.1).
//
// Pins the contract that survives the split from `Chip`: no label, a tone
// resolved through the shared vocabulary rather than spelled here (F27), and
// the glow that is the only reason this component is not a `Chip` with a
// smaller dot.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { StatusBadge } from './StatusBadge';

function badge(): HTMLElement {
  return screen.getByTestId('status-badge');
}

describe('StatusBadge', () => {
  it('renders no label of its own', () => {
    const { container } = render(<StatusBadge status="running" />);

    expect(badge()).toBeInTheDocument();
    expect(container.textContent).toBe('');
  });

  it('maps a machine status outside the run vocabulary to its own tone', () => {
    render(<StatusBadge status="idle" />);

    expect(badge()).toHaveAttribute('data-tone', 'emerald');
    expect(badge()).toHaveClass('bg-emerald-500');
  });

  it('defers to the shared run-status vocabulary for everything else', () => {
    render(<StatusBadge status="failed" />);

    expect(badge()).toHaveAttribute('data-tone', 'ruby');
    expect(badge()).toHaveClass('bg-ruby-500');
  });

  // Deliberately a non-run status: `runStatusMeta` lowercases internally, so a
  // run status proves nothing about the normalization this component owns —
  // only the raw `NON_RUN_TONES` lookup does.
  it('normalizes case before resolving the tone', () => {
    render(<StatusBadge status="IDLE" />);

    expect(badge()).toHaveAttribute('data-tone', 'emerald');
  });

  it('falls back to slate for a status the vocabulary has never seen', () => {
    render(<StatusBadge status="quantum_entangled" />);

    expect(badge()).toHaveAttribute('data-tone', 'slate');
    expect(badge()).toHaveClass('bg-slate-500');
  });

  it('glows for a live tone and stays flat for an inert one', () => {
    const { rerender } = render(<StatusBadge status="running" />);
    expect(badge().className).toContain('shadow-[');

    rerender(<StatusBadge status="cancelled" />);
    expect(badge().className).not.toContain('shadow-[');
  });

  it('merges a caller-supplied className', () => {
    render(<StatusBadge status="running" className="ml-1" />);

    expect(badge()).toHaveClass('ml-1');
  });
});
