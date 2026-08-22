// A smoke test over the modal's own wiring — the parts `decomposeReview.test.ts`
// proves in isolation, proved again against the DOM they drive. `dev:tauri` is
// the only other place this would surface, and a modal that throws on mount
// surfaces there as a blank window.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { DecomposeModal } from './DecomposeModal';
import { indexTickets } from '../../lib/ticketPresentation';
import type { ChangeKind, DecomposeProposal, ProposedChange, TicketView } from '../../types';

vi.mock('../../lib/discovery', () => ({ applyDecomposition: vi.fn() }));

afterEach(cleanup);

function change(id: string, kind: ChangeKind, extra: Partial<ProposedChange> = {}): ProposedChange {
  return {
    id,
    kind,
    seq: null,
    title: `ticket ${id}`,
    why: null,
    workflow_name: null,
    agent_kind: null,
    blocked_by: [],
    fields: [],
    ...extra,
  };
}

function proposal(extra: Partial<DecomposeProposal> = {}): DecomposeProposal {
  return {
    discovery_id: 'dsc-1',
    first_pass: false,
    tickets: [],
    changes: [change('revoke', 'added'), change('lease', 'added', { blocked_by: ['revoke'] })],
    locked: [],
    refused: [],
    refusal: null,
    violations: [],
    cost_usd: 0,
    tokens: 0,
    ...extra,
  };
}

function renderModal(subject: DecomposeProposal, tickets: TicketView[] = []) {
  const onApplied = vi.fn();
  render(
    <DecomposeModal
      proposal={subject}
      index={indexTickets(tickets)}
      onClose={() => {}}
      onApplied={onApplied}
    />,
  );
  return { onApplied };
}

describe('the footer label', () => {
  it('starts at every change and follows the checkboxes down', () => {
    renderModal(proposal());

    expect(screen.getByTestId('decompose-apply').textContent).toBe('Apply 2 of 2 changes');

    fireEvent.click(screen.getByText('ticket lease'));

    expect(screen.getByTestId('decompose-apply').textContent).toBe('Apply 1 of 2 changes');
  });

  it('disables the apply at zero rather than offering a no-op', () => {
    renderModal(proposal());

    fireEvent.click(screen.getByText('ticket lease'));
    fireEvent.click(screen.getByText('ticket revoke'));

    expect(screen.getByTestId('decompose-apply').hasAttribute('disabled')).toBe(true);
  });
});

describe('a refused subset', () => {
  it('lands on the checkboxes that caused it, not only in a message', async () => {
    const { applyDecomposition } = await import('../../lib/discovery');
    vi.mocked(applyDecomposition).mockRejectedValue(
      "these changes cannot be applied together: ticket 'lease' is blocked_by 'revoke', which is " +
        'not a ticket in this plan.',
    );
    renderModal(proposal());

    fireEvent.click(screen.getByText('ticket revoke'));
    fireEvent.click(screen.getByTestId('decompose-apply'));

    await waitFor(() => expect(screen.getByTestId('decompose-refusal')).toBeTruthy());
    for (const card of screen.getAllByTestId('decompose-change')) {
      expect(card.className).toContain('border-ruby-500/40');
    }
  });
});

describe('a proposal nothing survived', () => {
  it('says so in the validation bar and refuses to apply any of it', () => {
    renderModal(proposal({ refusal: 'it kept re-authoring the same graph' }));

    expect(screen.getByTestId('decompose-validation').textContent).toContain('Schema refused');
    expect(screen.getByTestId('decompose-apply').hasAttribute('disabled')).toBe(true);
  });
});
