// `docs/PRD_DISCOVERY.md` §5.4 locks a Ticket the moment it has a Feature.
// The drawer has to *show* that rather than take the edit and let the backend
// refuse it a round trip later, with the user's typing thrown away — so these
// pin the read-only rendering and the absence of a save at all.
//
// The unlocked half pins the other rule the wire depends on: `ticket_update`
// takes the whole ticket, because Rust reads an absent key and an explicit
// `null` identically.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TicketEditorDrawer } from './TicketEditorDrawer';
import { indexTickets } from '../../lib/ticketPresentation';
import type { DiscoveryBoard, Ticket, TicketView } from '../../types';

vi.mock('../../lib/discovery', () => ({
  getTicketBriefing: vi.fn(async () => 'DSC-2 has not landed.'),
  updateTicket: vi.fn(async () => ({ tickets: [], progress: EMPTY_PROGRESS }) as DiscoveryBoard),
  addTicketAttachment: vi.fn(),
  removeTicketAttachment: vi.fn(),
}));

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: vi.fn(async () => []),
  modelSupportsImages: vi.fn(() => true),
}));

vi.mock('../../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents: [{ kind: 'claude-code' }] }),
  effortLevelsFor: () => ['low', 'medium', 'high'],
}));

const EMPTY_PROGRESS = { blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 0, live: 0 };

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

function renderDrawer(subject: TicketView) {
  const onSaved = vi.fn();
  render(
    <TicketEditorDrawer
      view={subject}
      index={indexTickets([subject])}
      siblings={[subject]}
      workflows={[]}
      machineId="local"
      busy={false}
      onClose={() => {}}
      onSaved={onSaved}
      onRefresh={() => {}}
      onStart={() => {}}
      onForceStart={() => {}}
      onDrop={() => {}}
    />,
  );
  return { onSaved };
}

describe('a locked ticket', () => {
  it('is shown as locked, with no save to fail', () => {
    renderDrawer(view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'));

    expect(screen.getByTestId('ticket-locked')).toBeTruthy();
    expect(screen.queryByTestId('ticket-save')).toBeNull();
  });

  it('locks on the feature id alone, before the state has caught up', () => {
    renderDrawer(view(ticket({ feature_id: 'f-1' })));

    expect(screen.getByTestId('ticket-locked')).toBeTruthy();
  });

  it('leaves every field read-only', () => {
    renderDrawer(view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'));

    expect(screen.getByLabelText('Title').hasAttribute('disabled')).toBe(true);
    expect(screen.getByLabelText('Description').hasAttribute('disabled')).toBe(true);
    expect(screen.getByLabelText('Acceptance 1').hasAttribute('disabled')).toBe(true);
  });

  it('offers no dropzone — its attachments went to the feature when it started', () => {
    renderDrawer(view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'));

    expect(screen.queryByText(/or drop here/)).toBeNull();
  });
});

describe('an unstarted ticket', () => {
  it('says every field is yours, and offers a save once something changes', () => {
    renderDrawer(view(ticket()));

    expect(screen.queryByTestId('ticket-locked')).toBeNull();
    const save = screen.getByTestId('ticket-save');
    expect(save.hasAttribute('disabled')).toBe(true);

    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'A new title' } });

    expect(save.hasAttribute('disabled')).toBe(false);
  });

  it('saves the whole ticket, every key present', async () => {
    const { updateTicket } = await import('../../lib/discovery');
    renderDrawer(view(ticket()));

    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'A new title' } });
    fireEvent.click(screen.getByTestId('ticket-save'));

    await waitFor(() => expect(updateTicket).toHaveBeenCalled());
    expect(vi.mocked(updateTicket).mock.calls[0][1]).toEqual({
      title: 'A new title',
      description: 'Every client watches its own runs down a single connection.',
      acceptance: ['Two clients stream concurrently without interleaving'],
      files: ['crates/demeteo-runner/src/stream/mux.rs'],
      blocked_by: [],
      test_command: 'npm run checks:code',
      workflow_id: null,
      agent_kind: 'claude-code',
      model: 'opus',
      effort: 'high',
    });
  });

  it('takes the board the save returns rather than patching a row', async () => {
    const { onSaved } = renderDrawer(view(ticket()));

    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'A new title' } });
    fireEvent.click(screen.getByTestId('ticket-save'));

    await waitFor(() => expect(onSaved).toHaveBeenCalledWith({ tickets: [], progress: EMPTY_PROGRESS }));
  });

  it('shows what its agent will be told, composed by the backend', async () => {
    renderDrawer(view(ticket()));

    await waitFor(() => expect(screen.getByText('DSC-2 has not landed.')).toBeTruthy());
  });
});
