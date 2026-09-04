// `TicketOverlayPanel`'s backdrop is pointer-events-none (§3.2.1), so once the
// inspector floats as an overlay in 'overlay-inspector'/'stacked' mode, Escape
// is its only dismiss path unless the panel itself carries a visible control.
// This pins that control the same way `TicketEditorDrawer.test.tsx` would pin
// its own Close/Discard button, mirroring the pattern this ticket copies.
//
// The description is model-authored (`docs/PRD_DISCOVERY.md`'s discovery
// interview writes it), so it renders through `AgentMarkdown` rather than as
// plain text — see `AgentMarkdown.test.tsx` for why these render the real
// react-markdown instead of stubbing it.

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
    description: 'Every client watches its own runs down a single connection.',
    acceptance: ['Two clients stream concurrently without interleaving'],
    files: ['crates/demeteo-runner/src/stream/mux.rs'],
    blocked_by: [],
    test_command: 'npm run checks:code',
    workflow_id: null,
    agent_kind: 'claude-code',
    model: 'opus',
    effort: 'high',
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

function renderInspector(subject: TicketView) {
  render(
    <TicketInspector
      view={subject}
      index={indexTickets([subject])}
      workflowName={null}
      onStart={() => {}}
      onForceStart={() => {}}
      onEdit={() => {}}
      onOpenFeature={() => {}}
      onClose={() => {}}
      busy={false}
    />,
  );
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

describe('the ticket description', () => {
  it('renders bold Markdown as a strong element, not literal asterisks', () => {
    renderInspector(
      view(ticket({ description: 'Do not commit the six `.dc.html` **artboards**.' })),
    );

    const strong = screen.getByText('artboards');
    expect(strong.tagName).toBe('STRONG');
    expect(screen.getByTestId('agent-markdown').textContent).not.toContain('**');
  });

  it('renders nothing when the description is empty', () => {
    renderInspector(view(ticket({ description: '' })));

    expect(screen.queryByTestId('agent-markdown')).toBeNull();
  });

  it('renders an embedded HTML tag as inert text, not a real element', () => {
    renderInspector(
      view(ticket({ description: '<img src="x" onerror="boom"> in the description' })),
    );

    expect(screen.queryByRole('img')).toBeNull();
    expect(screen.getByTestId('agent-markdown').textContent).toContain(
      '<img src="x" onerror="boom">',
    );
  });

  it('renders visually muted relative to the ticket title', () => {
    renderInspector(
      view(ticket({ description: 'Every client watches its own runs down a single connection.' })),
    );

    const paragraph = screen.getByTestId('agent-markdown').querySelector('p');
    expect(paragraph).not.toBeNull();
    expect(paragraph?.className).toContain('text-slate-400');
  });
});
