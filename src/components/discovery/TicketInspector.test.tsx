// `TicketOverlayPanel`'s backdrop is pointer-events-none (§3.2.1), so once the
// inspector floats as an overlay in 'overlay-inspector'/'stacked' mode, Escape
// is its only dismiss path unless the panel itself carries a visible control.
// This pins that control the same way `TicketEditorModal.test.tsx` would pin
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
import type { Machine, Ticket, TicketView } from '../../types';
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
    machine_id: null,
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
    placement: { placement: { kind: 'local' }, inherited: true },
    feature: null,
  };
}

const BUILD_BOX: Machine = {
  id: 'm-build',
  name: 'Build box',
  host: 'build.internal',
  port: 22,
  username: 'dev',
  auth_type: 'key',
};

function renderInspector(subject: TicketView, machines: readonly Machine[] = []) {
  render(
    <TicketInspector
      view={subject}
      index={indexTickets([subject])}
      workflowName={null}
      machines={machines}
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
        machines={[]}
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

describe('the placement chip', () => {
  it('marks a placement inherited from the Discovery default', () => {
    renderInspector(view(ticket()));

    expect(screen.getByText('Local · default')).toBeInTheDocument();
  });

  it('names the machine of an explicit detached placement, without the marker', () => {
    const subject: TicketView = {
      ...view(ticket({ machine_id: 'm-build' })),
      placement: { placement: { kind: 'detached', machine_id: 'm-build' }, inherited: false },
    };

    renderInspector(subject, [BUILD_BOX]);

    expect(screen.getByText('Detached · Build box')).toBeInTheDocument();
    expect(screen.queryByText(/· default/)).toBeNull();
  });

  it('shows the run mirror status beside a detached attempt', () => {
    const subject: TicketView = {
      ...view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'),
      placement: { placement: { kind: 'detached', machine_id: 'm-gone' }, inherited: true },
      feature: {
        id: 'f-1',
        status: 'running',
        mr_state: null,
        mr_url: null,
        placement: { kind: 'detached', machine_id: 'm-gone' },
        remote: { machine_id: 'm-gone', run_id: 'r-1', status: 'running' },
      },
    };

    renderInspector(subject);

    expect(screen.getByText('Detached · m-gone')).toBeInTheDocument();
    expect(screen.getByTestId('ticket-run-mirror').textContent).toContain('Running');
  });

  it('names where a started attempt ran, not the stored placement', () => {
    const subject: TicketView = {
      ...view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'),
      feature: {
        id: 'f-1',
        status: 'running',
        mr_state: null,
        mr_url: null,
        placement: { kind: 'detached', machine_id: 'm-build' },
        remote: { machine_id: 'm-build', run_id: 'r-1', status: 'running' },
      },
    };

    renderInspector(subject, [BUILD_BOX]);

    expect(screen.getByText('Detached · Build box')).toBeInTheDocument();
    expect(screen.queryByText('Local · default')).toBeNull();
  });

  it('names a detached attempt whose mirror row is missing by its recorded placement', () => {
    const subject: TicketView = {
      ...view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'),
      feature: {
        id: 'f-1',
        status: 'running',
        mr_state: null,
        mr_url: null,
        placement: { kind: 'detached', machine_id: 'm-build' },
        remote: null,
      },
    };

    renderInspector(subject, [BUILD_BOX]);

    expect(screen.getByText('Detached · Build box')).toBeInTheDocument();
    expect(screen.queryByText(/^Local/)).toBeNull();
  });

  it('labels a started attempt with no recorded placement unknown, never Local', () => {
    const subject: TicketView = {
      ...view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'),
      feature: {
        id: 'f-1',
        status: 'running',
        mr_state: null,
        mr_url: null,
        placement: null,
        remote: null,
      },
    };

    renderInspector(subject, [BUILD_BOX]);

    expect(screen.getByText('Placement unknown')).toBeInTheDocument();
    expect(screen.queryByText(/^Local/)).toBeNull();
  });

  it('labels a started local attempt Local, without the marker', () => {
    const subject: TicketView = {
      ...view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'),
      placement: { placement: { kind: 'detached', machine_id: 'm-build' }, inherited: true },
      feature: {
        id: 'f-1',
        status: 'running',
        mr_state: null,
        mr_url: null,
        placement: { kind: 'local' },
        remote: null,
      },
    };

    renderInspector(subject, [BUILD_BOX]);

    expect(screen.getByText('Local')).toBeInTheDocument();
    expect(screen.queryByText(/Detached/)).toBeNull();
  });
});
