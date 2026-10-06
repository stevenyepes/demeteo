// The claim this file defends: a stored pass that stopped before producing a
// plan is offered as something to ask for again, never as something to review
// — the modal behind Review has no plan to show for it.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { DecomposeProposal } from '../../types';
import { PendingProposalNotice } from './PendingProposalNotice';

function proposal(extra: Partial<DecomposeProposal> = {}): DecomposeProposal {
  return {
    discovery_id: 'dsc-1',
    first_pass: true,
    tickets: [],
    changes: [],
    locked: [],
    refused: [],
    refusal: null,
    violations: [],
    cost_usd: 0,
    tokens: 0,
    ...extra,
  };
}

function renderNotice(p: DecomposeProposal) {
  const handlers = { onReview: vi.fn(), onDiscard: vi.fn(), onDecompose: vi.fn() };
  render(<PendingProposalNotice proposal={p} busy={false} {...handlers} />);
  return handlers;
}

describe('PendingProposalNotice', () => {
  it('offers a stopped pass again instead of a review', async () => {
    const handlers = renderNotice(proposal({ stopped: 'Agent blocked: no output for 600s' }));

    expect(screen.queryByTestId('pending-proposal-review')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Decompose again' }));
    expect(handlers.onDecompose).toHaveBeenCalledOnce();
    expect(handlers.onReview).not.toHaveBeenCalled();
  });

  it('dismisses a stopped pass through discard', async () => {
    const handlers = renderNotice(proposal({ stopped: 'Agent blocked: no output for 600s' }));

    await userEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(handlers.onDiscard).toHaveBeenCalledOnce();
  });

  it('offers a finished pass for review', () => {
    renderNotice(proposal());

    expect(screen.getByTestId('pending-proposal-review')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Decompose again' })).toBeNull();
  });
});
