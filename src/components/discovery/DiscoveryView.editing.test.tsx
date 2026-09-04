// Bug repro: `editingId` and `selectedId` are independent state in
// `DiscoveryView`. Clicking a different ticket card only updates
// `selectedId` — nothing clears `editingId` — so the editor drawer keeps
// showing the ticket that was open for edit no matter which card is clicked.

import { fireEvent, render, waitFor, within } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Discovery, DiscoveryBoard, DiscoveryDetail, Ticket, TicketView } from '../../types';
import { DiscoveryView } from './DiscoveryView';

const discovery: Discovery = {
  id: 'd-1',
  project_id: 'p-1',
  title: 'multi-client runner',
  status: 'open',
  machine_id: 'local',
  agent_kind: 'claude-code',
  model: null,
  effort: null,
  resume_session_id: null,
  worktree_path: null,
  attachments: [],
  total_cost: 0,
  tokens: 0,
  created_at: 0,
  updated_at: 0,
};

const detail: DiscoveryDetail = {
  discovery,
  messages: [],
  pending_proposal: null,
  turn_running: false,
};

function ticket(id: string, seq: number, title: string): Ticket {
  return {
    id,
    discovery_id: 'd-1',
    seq,
    title,
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
    force_started_at: 0,
    feature_id: null,
    created_at: 0,
    updated_at: 0,
  };
}

function makeView(id: string, seq: number, title: string): TicketView {
  return {
    ticket: ticket(id, seq, title),
    standing: { id, lane: 'ready', startable: true, blockers: [] },
    feature: null,
  };
}

const board: DiscoveryBoard = {
  tickets: [makeView('t-1', 1, 'First ticket'), makeView('t-2', 2, 'Second ticket')],
  progress: { blocked: 0, ready: 2, in_flight: 0, landed: 0, dropped: 0, live: 2 },
};

beforeEach(() => {
  vi.mocked(listen).mockImplementation(async () => () => {});
  vi.mocked(invoke).mockImplementation((command: string) => {
    switch (command) {
      case 'discovery_get':
        return Promise.resolve(detail);
      case 'discovery_board':
        return Promise.resolve(board);
      case 'ticket_briefing':
        return Promise.resolve('what the agent will be told');
      case 'workflow_list':
      case 'get_machines':
      case 'list_agents':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
});

describe('switching tickets while one is open for edit', () => {
  it('does not keep the editor open on the ticket that is no longer selected', async () => {
    const view = render(<DiscoveryView discoveryId="d-1" discoveryTitle="multi-client runner" />);
    await view.findByPlaceholderText(/./);

    fireEvent.click(view.getByRole('radio', { name: 'Board' }));

    const boardEl = await view.findByTestId('ticket-board');
    const firstCard = within(boardEl).getByText('First ticket').closest('button');
    if (!firstCard) throw new Error('first ticket card did not render');
    fireEvent.click(firstCard);

    fireEvent.click(await view.findByTestId('ticket-edit'));
    const editor = await view.findByTestId('ticket-editor');
    expect(within(editor).getByLabelText('Title')).toHaveValue('First ticket');

    const secondCard = within(boardEl).getByText('Second ticket').closest('button');
    if (!secondCard) throw new Error('second ticket card did not render');
    fireEvent.click(secondCard);

    // Clicking a different ticket must not leave the first ticket's editor
    // open — the user is now looking at the second ticket in the list, but
    // the drawer they see should not still be editing the first one.
    await waitFor(() => {
      const editorStillOpen = view.queryByTestId('ticket-editor');
      if (editorStillOpen) {
        expect(within(editorStillOpen).getByLabelText('Title')).not.toHaveValue('First ticket');
      }
    });
  });
});
