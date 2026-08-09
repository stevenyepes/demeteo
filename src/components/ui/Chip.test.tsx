// Unit tests for the Chip primitive (UI redesign plan §5.1).
//
// Pins the three things a call site is no longer allowed to spell for
// itself: tone resolution through `lib/runStatus.ts` (F27), the pulse
// landing on the dot rather than the pill, and the fallback for a status
// string the vocabulary has never seen.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { TONE_CHIP } from '../../lib/runStatus';
import { Chip } from './Chip';

function chip(): HTMLElement {
  return screen.getByTestId('chip');
}

describe('Chip', () => {
  it('resolves tone and label from a status string', () => {
    render(<Chip status="awaiting_gate" />);

    expect(chip()).toHaveTextContent('Gate needs you');
    expect(chip()).toHaveAttribute('data-tone', 'amber');
    for (const cls of TONE_CHIP.amber.split(' ')) {
      expect(chip()).toHaveClass(cls);
    }
  });

  it('lets an explicit tone win over the status tone', () => {
    render(<Chip status="failed" tone="cyan" />);

    expect(chip()).toHaveAttribute('data-tone', 'cyan');
    expect(chip()).toHaveClass('text-cyan-400');
    expect(chip()).not.toHaveClass('text-ruby-400');
  });

  it('falls back to slate with a humanized label for an unknown status', () => {
    render(<Chip status="quantum_entangled" />);

    expect(chip()).toHaveAttribute('data-tone', 'slate');
    expect(chip()).toHaveTextContent('quantum entangled');
    expect(chip()).toHaveClass('text-slate-400');
  });

  it('prefers children over the resolved status label', () => {
    render(<Chip status="running">Live tail</Chip>);

    expect(chip()).toHaveTextContent('Live tail');
    expect(chip()).not.toHaveTextContent('Running');
  });

  it('pulses the dot and never the container for a live status', () => {
    render(<Chip status="running" />);

    expect(screen.getByTestId('chip-dot')).toHaveClass('animate-pulse');
    expect(chip()).not.toHaveClass('animate-pulse');
  });

  it('renders no dot for an inert status', () => {
    render(<Chip status="completed" />);

    expect(screen.queryByTestId('chip-dot')).toBeNull();
  });

  it('shows a pulsing dot when pulse is forced on an inert status', () => {
    render(<Chip status="cancelled" pulse />);

    expect(screen.getByTestId('chip-dot')).toHaveClass('animate-pulse');
    expect(chip()).not.toHaveClass('animate-pulse');
  });

  it('renders a still dot when the dot is asked for without a pulse', () => {
    render(<Chip tone="violet" dot>Connected</Chip>);

    expect(screen.getByTestId('chip-dot')).not.toHaveClass('animate-pulse');
  });

  it('suppresses the dot on a live status when the caller opts out', () => {
    render(<Chip status="verifying" dot={false} />);

    expect(screen.queryByTestId('chip-dot')).toBeNull();
    expect(chip()).not.toHaveClass('animate-pulse');
  });

  it('renders a leading icon before the label', () => {
    render(<Chip tone="cyan" icon={<span data-testid="chip-test-icon" />}>Remote</Chip>);

    expect(screen.getByTestId('chip-test-icon')).toBeInTheDocument();
  });

  it('applies the small size variant', () => {
    render(<Chip status="running" size="sm" />);

    expect(chip()).toHaveClass('text-[10px]');
    expect(chip()).not.toHaveClass('text-xs');
  });

  it('exposes a tooltip through the title attribute', () => {
    render(<Chip tone="violet" title="Workflow: Standard Feature Pipeline">Standard Feature Pipeline</Chip>);

    expect(chip()).toHaveAttribute('title', 'Workflow: Standard Feature Pipeline');
  });

  it('truncates the label under a caller-set max width', () => {
    render(
      <Chip tone="violet" maxWidth="220px">
        A workflow name long enough to need the cap
      </Chip>,
    );

    expect(chip()).toHaveStyle({ maxWidth: '220px' });
    expect(screen.getByTestId('chip-label')).toHaveClass('truncate');
  });

  it('leaves the label untruncated when no max width is set', () => {
    render(<Chip tone="violet">Short</Chip>);

    expect(screen.getByTestId('chip-label')).not.toHaveClass('truncate');
  });

  it('merges a caller-supplied className', () => {
    render(<Chip status="running" className="ml-1" />);

    expect(chip()).toHaveClass('ml-1');
  });
});
