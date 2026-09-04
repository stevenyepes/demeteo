// `TicketOverlayPanel`'s backdrop is pointer-events-none (§3.2.1), so once the
// inspector floats as an overlay in 'overlay-inspector'/'stacked' mode, Escape
// is its only dismiss path unless the panel itself carries a visible control.
// This pins that control the same way `TicketEditorDrawer.test.tsx` would pin
// its own Close/Discard button, mirroring the pattern this ticket copies.

import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { indexTickets } from '../../lib/ticketPresentation';
import type { Ticket, TicketView } from '../../types';
import { TicketInspector } from './TicketInspector';

vi.mock('../../lib/discovery', () => ({
  getTicketBriefing: vi.fn(async () => 'DSC-3 has not landed.'),
}));

afterEach(cleanup);

function ticket(extra: Partial<Ticket> = {}): Ticket {
  return {
    id: 't3',
    discovery_id: 'dsc-1',
    seq: 3,
    title: 'Multiplex run streams over one connection',
    description: '',
    acceptance: [],
    files: [],
    blocked_by: [],
    test_command: null,
    workflow_id: null,
    agent_kind: null,
    model: null,
    effort: null,
    attachments: [],
    state: 'unstarted',
    drop_reason: null,
    force_start_reason: null,
    force_started_at: null,
    feature_id: null,
    created_at: 0,
    updated_at: 0,
    ...extra,
  };
}

function view(row: Ticket, lane: TicketView['standing']['lane'] = 'ready'): TicketView {
  return {
    ticket: row,
    standing: { id: row.id, lane, startable: lane === 'ready', blockers: [] },
    feature: null,
  };
}

describe('TicketInspector', () => {
  it('renders a Close button in its sub-header and calls onClose when clicked', async () => {
    const user = userEvent.setup();
    const subject = view(ticket());
    const onClose = vi.fn();

    render(
      <TicketInspector
        view={subject}
        index={indexTickets([subject])}
        workflowName={null}
        busy={false}
        onStart={() => {}}
        onForceStart={() => {}}
        onEdit={() => {}}
        onOpenFeature={() => {}}
        onClose={onClose}
      />,
    );

    const close = screen.getByRole('button', { name: 'Close' });
    expect(close).toBeInTheDocument();

    await user.click(close);

    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
