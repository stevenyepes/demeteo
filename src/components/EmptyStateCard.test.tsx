// Smoke tests for the EmptyStateCard first-run landing card.
//
// Spec finding C-4: the wizard entry tile for `create-project` must live on the
// empty-state card alongside Connect Providers / Sync Worktrees / Deploy Agents,
// and clicking it must fire only its own handler — no cross-firing into the
// legacy three.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import EmptyStateCard from './EmptyStateCard';

function renderCard() {
  const handlers = {
    onSeedSample: vi.fn(),
    onConnectProviders: vi.fn(),
    onSyncWorktrees: vi.fn(),
    onDeployAgents: vi.fn(),
    onCreateFromZero: vi.fn(),
  };

  render(<EmptyStateCard {...handlers} />);

  return handlers;
}

describe('EmptyStateCard', () => {
  it('renders all four tiles', () => {
    renderCard();

    for (const label of [
      'Connect Providers',
      'Sync Worktrees',
      'Deploy Agents',
      'Create from Scratch',
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  // The stable test id is what downstream selectors key off.
  it('gives the create-project tile a stable test id', () => {
    renderCard();

    expect(screen.getByTestId('empty-state-create-project')).toBeInTheDocument();
  });

  it('fires only onCreateFromZero when the create-project tile is clicked', async () => {
    const handlers = renderCard();

    await userEvent.click(screen.getByTestId('empty-state-create-project'));

    expect(handlers.onCreateFromZero).toHaveBeenCalledTimes(1);
    expect(handlers.onSeedSample).not.toHaveBeenCalled();
    expect(handlers.onConnectProviders).not.toHaveBeenCalled();
    expect(handlers.onSyncWorktrees).not.toHaveBeenCalled();
    expect(handlers.onDeployAgents).not.toHaveBeenCalled();
  });

  it('still renders the legacy tiles as buttons', () => {
    renderCard();

    // 3 legacy tiles + the new tile + seed sample.
    expect(screen.getAllByRole('button').length).toBeGreaterThanOrEqual(4);
  });
});
