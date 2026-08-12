// Behaviour tests for the HeaderNavItem primitive (UI_REDESIGN_PLAN §5.1).
//
// The load-bearing case is the icons density: the label text node has to be
// absent from the DOM rather than hidden with CSS, while the button keeps the
// same accessible name. A class-only implementation passes an eyeball check and
// fails here, which is the point.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sliders } from 'lucide-react';
import { describe, expect, it, vi } from 'vitest';

import { HeaderNavItem } from './HeaderNavItem';

describe('HeaderNavItem', () => {
  it('renders the label at labels density and calls onClick when clicked', async () => {
    const onClick = vi.fn();
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Workflows"
        density="labels"
        testId="topbar-workflows"
        onClick={onClick}
      />,
    );

    expect(screen.getByText('Workflows')).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('topbar-workflows'));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('drops the label text node at icons density while keeping the accessible name', async () => {
    const onClick = vi.fn();
    render(
      <HeaderNavItem icon={Sliders} label="Workflows" density="icons" onClick={onClick} />,
    );

    expect(screen.queryByText('Workflows')).toBeNull();

    const button = screen.getByRole('button', { name: 'Workflows' });
    expect(button).toBeInTheDocument();

    await userEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('prefers the count badge over the activity dot', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="labels"
        count={2}
        activity
        pulse
        testId="topbar-runs"
        pulseTestId="topbar-runs-pulse"
        onClick={() => {}}
      />,
    );

    expect(screen.getByTestId('topbar-runs-badge')).toHaveTextContent('2');
    expect(screen.queryByTestId('topbar-runs-pulse')).toBeNull();
  });

  it('prefers the activity dot over the pulse dot', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="labels"
        count={0}
        activity
        pulse
        testId="topbar-runs"
        pulseTestId="topbar-runs-pulse"
        onClick={() => {}}
      />,
    );

    expect(screen.queryByTestId('topbar-runs-badge')).toBeNull();
    expect(screen.getByTestId('topbar-runs-pulse')).toHaveAttribute('data-badge', 'activity');
  });

  it('falls back to the pulse dot when there is no count and no activity', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Terminals"
        density="labels"
        pulse
        pulseTestId="topbar-terminal-pulse"
        onClick={() => {}}
      />,
    );

    expect(screen.getByTestId('topbar-terminal-pulse')).toHaveAttribute('data-badge', 'pulse');
  });

  it('renders no badge at all without a count, activity or pulse', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="labels"
        count={0}
        testId="topbar-runs"
        pulseTestId="topbar-runs-pulse"
        onClick={() => {}}
      />,
    );

    expect(screen.queryByTestId('topbar-runs-badge')).toBeNull();
    expect(screen.queryByTestId('topbar-runs-pulse')).toBeNull();
  });

  it('caps the count badge at 9+', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="icons"
        count={12}
        testId="topbar-runs"
        onClick={() => {}}
      />,
    );

    expect(screen.getByTestId('topbar-runs-badge')).toHaveTextContent('9+');
  });

  it('marks the active item via data-active and aria-current', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Workflows"
        density="labels"
        active
        testId="topbar-workflows"
        onClick={() => {}}
      />,
    );

    const button = screen.getByTestId('topbar-workflows');
    expect(button).toHaveAttribute('data-active', 'true');
    expect(button).toHaveAttribute('aria-current', 'page');
  });

  it('leaves aria-current unset when inactive', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Workflows"
        density="labels"
        testId="topbar-workflows"
        onClick={() => {}}
      />,
    );

    const button = screen.getByTestId('topbar-workflows');
    expect(button).toHaveAttribute('data-active', 'false');
    expect(button).not.toHaveAttribute('aria-current');
  });

  // `aria-label` overrides contents, so the badge's text node cannot reach a
  // screen reader on its own — at `icons` density there is no text node to
  // reach it through either. Both densities have to publish the count.
  it('folds the count into the accessible name at both densities', () => {
    const { rerender } = render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="labels"
        count={3}
        testId="topbar-runs"
        onClick={() => {}}
      />,
    );

    expect(screen.getByRole('button', { name: /3/ })).toBe(screen.getByTestId('topbar-runs'));
    expect(screen.getByTestId('topbar-runs')).toHaveAccessibleName('Runs 3');

    rerender(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="icons"
        count={3}
        testId="topbar-runs"
        onClick={() => {}}
      />,
    );

    expect(screen.getByRole('button', { name: /3/ })).toBe(screen.getByTestId('topbar-runs'));
    expect(screen.getByTestId('topbar-runs')).toHaveAccessibleName('Runs 3');
  });

  it('announces the capped badge text rather than the raw count', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="icons"
        count={12}
        testId="topbar-runs"
        onClick={() => {}}
      />,
    );

    expect(screen.getByTestId('topbar-runs')).toHaveAccessibleName('Runs 9+');
  });

  it('leaves the accessible name unadorned when there is no count', () => {
    const { rerender } = render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="labels"
        testId="topbar-runs"
        onClick={() => {}}
      />,
    );

    expect(screen.getByTestId('topbar-runs')).toHaveAccessibleName('Runs');

    rerender(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="icons"
        count={0}
        testId="topbar-runs"
        onClick={() => {}}
      />,
    );

    expect(screen.getByTestId('topbar-runs')).toHaveAccessibleName('Runs');
  });

  it('keeps the accessible name when a longer title is supplied', () => {
    render(
      <HeaderNavItem
        icon={Sliders}
        label="Runs"
        density="icons"
        title="Runs — 2 runs need attention"
        testId="topbar-runs"
        onClick={() => {}}
      />,
    );

    const button = screen.getByRole('button', { name: 'Runs' });
    expect(button).toHaveAttribute('title', 'Runs — 2 runs need attention');
  });

  it('publishes an overriding accessible name at both densities', () => {
    const { rerender } = render(
      <HeaderNavItem
        icon={Sliders}
        label="Terminals"
        density="labels"
        ariaLabel="Open terminals view"
        testId="topbar-terminal-toggle"
        onClick={() => {}}
      />,
    );

    expect(screen.getByText('Terminals')).toBeInTheDocument();
    expect(screen.getByTestId('topbar-terminal-toggle')).toHaveAccessibleName('Open terminals view');

    rerender(
      <HeaderNavItem
        icon={Sliders}
        label="Terminals"
        density="icons"
        ariaLabel="Open terminals view"
        testId="topbar-terminal-toggle"
        onClick={() => {}}
      />,
    );

    expect(screen.queryByText('Terminals')).toBeNull();
    expect(screen.getByTestId('topbar-terminal-toggle')).toHaveAccessibleName('Open terminals view');
  });
});
