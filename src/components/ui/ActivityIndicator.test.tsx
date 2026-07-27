// Unit tests for the ActivityIndicator presentational primitive (spec
// `TERMINAL_ACTIVITY` §2). One assertion per state, including that the
// null state renders nothing at all.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { ActivityIndicator } from './ActivityIndicator';

describe('ActivityIndicator', () => {
  it('renders nothing for the null state', () => {
    const { container } = render(<ActivityIndicator activity={null} />);
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByTestId('activity-indicator')).not.toBeInTheDocument();
  });

  it('renders a violet animated spinner while working', () => {
    render(<ActivityIndicator activity="working" />);

    const mark = screen.getByTestId('activity-indicator');
    expect(mark).toHaveAttribute('data-activity', 'working');
    expect(mark).toHaveAccessibleName('Working');
    expect(mark).toHaveClass('text-violet-300');
    // The spinner animates.
    expect(mark.querySelector('.animate-spin')).not.toBeNull();
  });

  it('renders a steady amber dot when awaiting input', () => {
    render(<ActivityIndicator activity="awaiting_input" />);

    const mark = screen.getByTestId('activity-indicator');
    expect(mark).toHaveAttribute('data-activity', 'awaiting_input');
    expect(mark).toHaveAccessibleName('Waiting for you');
    const dot = mark.querySelector('span[aria-hidden="true"]');
    expect(dot).not.toBeNull();
    expect(dot).toHaveClass('bg-amber-400');
    // A plain "your turn" dot does not pulse.
    expect(dot).not.toHaveClass('animate-pulse-glow');
  });

  it('renders a pulsing red-amber dot when awaiting approval (highest salience)', () => {
    render(<ActivityIndicator activity="awaiting_approval" />);

    const mark = screen.getByTestId('activity-indicator');
    expect(mark).toHaveAttribute('data-activity', 'awaiting_approval');
    expect(mark).toHaveAccessibleName('Needs a decision');
    const dot = mark.querySelector('span[aria-hidden="true"]');
    expect(dot).not.toBeNull();
    expect(dot).toHaveClass('bg-ruby-400');
    expect(dot).toHaveClass('animate-pulse-glow');
  });
});
