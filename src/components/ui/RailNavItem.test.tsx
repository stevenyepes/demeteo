// Behavior tests for the RailNavItem primitive (TERMINALS_VIEW_SPEC §5, §6).
//
// These pin the load-bearing bits of the rail's "Terminals" entry: the label
// renders and clicks fire in the expanded variant, the count badge appears only
// when count > 0, active state surfaces via data-active / aria-current, and the
// collapsed variant drops the visible label while staying clickable through an
// accessible name (title / aria-label).

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { TerminalSquare } from 'lucide-react';
import { describe, expect, it, vi } from 'vitest';

import { RailNavItem } from './RailNavItem';

describe('RailNavItem', () => {
  it('renders the label in expanded mode and calls onClick when clicked', async () => {
    const onClick = vi.fn();
    render(<RailNavItem icon={TerminalSquare} label="Terminals" onClick={onClick} />);

    expect(screen.getByText('Terminals')).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('rail-nav-item'));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('renders the count badge when count > 0', () => {
    render(
      <RailNavItem icon={TerminalSquare} label="Terminals" count={3} onClick={() => {}} />,
    );

    expect(screen.getByText('3')).toBeInTheDocument();
  });

  it('hides the count badge when count is 0 or undefined', () => {
    const { rerender } = render(
      <RailNavItem icon={TerminalSquare} label="Terminals" count={0} onClick={() => {}} />,
    );
    expect(screen.queryByText('0')).not.toBeInTheDocument();

    rerender(<RailNavItem icon={TerminalSquare} label="Terminals" onClick={() => {}} />);
    // No numeric badge rendered at all.
    expect(screen.getByTestId('rail-nav-item')).toBeInTheDocument();
  });

  it('marks the active item via data-active and aria-current', () => {
    render(
      <RailNavItem icon={TerminalSquare} label="Terminals" active onClick={() => {}} />,
    );

    const button = screen.getByTestId('rail-nav-item');
    expect(button).toHaveAttribute('data-active', 'true');
    expect(button).toHaveAttribute('aria-current', 'page');
  });

  it('leaves aria-current unset when inactive', () => {
    render(<RailNavItem icon={TerminalSquare} label="Terminals" onClick={() => {}} />);

    const button = screen.getByTestId('rail-nav-item');
    expect(button).toHaveAttribute('data-active', 'false');
    expect(button).not.toHaveAttribute('aria-current');
  });

  it('hides the text label in collapsed mode but stays clickable with an accessible name', async () => {
    const onClick = vi.fn();
    render(
      <RailNavItem icon={TerminalSquare} label="Terminals" collapsed onClick={onClick} />,
    );

    // The visible text label is not rendered in the collapsed (icon-only) variant.
    expect(screen.queryByText('Terminals')).not.toBeInTheDocument();

    // ...but the button is still reachable by its accessible name and clickable.
    const button = screen.getByRole('button', { name: 'Terminals' });
    expect(button).toBeInTheDocument();

    await userEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('renders the count badge in collapsed mode when count > 0', () => {
    render(
      <RailNavItem
        icon={TerminalSquare}
        label="Terminals"
        collapsed
        count={5}
        onClick={() => {}}
      />,
    );

    expect(screen.getByText('5')).toBeInTheDocument();
  });

  it('hides the attention badge when attentionCount is 0 or undefined', () => {
    const { rerender } = render(
      <RailNavItem
        icon={TerminalSquare}
        label="Terminals"
        attentionCount={0}
        onClick={() => {}}
      />,
    );
    expect(screen.queryByTestId('rail-nav-attention')).not.toBeInTheDocument();

    rerender(<RailNavItem icon={TerminalSquare} label="Terminals" onClick={() => {}} />);
    expect(screen.queryByTestId('rail-nav-attention')).not.toBeInTheDocument();
  });

  it('renders the attention badge when attentionCount > 0 (expanded and collapsed)', () => {
    const { rerender } = render(
      <RailNavItem
        icon={TerminalSquare}
        label="Terminals"
        attentionCount={2}
        onClick={() => {}}
      />,
    );
    const badge = screen.getByTestId('rail-nav-attention');
    expect(badge).toHaveTextContent('2');

    rerender(
      <RailNavItem
        icon={TerminalSquare}
        label="Terminals"
        collapsed
        attentionCount={2}
        onClick={() => {}}
      />,
    );
    expect(screen.getByTestId('rail-nav-attention')).toHaveTextContent('2');
  });
});
