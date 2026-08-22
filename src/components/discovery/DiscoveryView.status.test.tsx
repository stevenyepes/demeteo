// The workspace's reading of `discovery_turn_status`. A status the handler does
// not know reads as a turn that has stopped, so a phase added to the wire and
// not added here does not merely go unrendered — it clears the composer the
// click just locked, which is worse than the silence it was added to fix.

import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { listen, type Event, type EventCallback } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  Discovery,
  DiscoveryBoard,
  DiscoveryDetail,
  DiscoveryMessage,
} from '../../types';
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

const board: DiscoveryBoard = {
  tickets: [],
  progress: { blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 0, live: 0 },
};

const stored: DiscoveryMessage = {
  id: 'm-1',
  discovery_id: 'd-1',
  role: 'user',
  content: 'what should this do?',
  cost_usd: null,
  tokens: null,
  activity: null,
  created_at: 0,
};

type StatusPayload = { discovery_id: string; status: string; reason: string | null };

/** The handlers the view registered, by event name. */
const listeners = new Map<string, EventCallback<unknown>>();

/** What `discovery_send_turn` answers, so a test can hold it open or reject
 *  it. The backend returns the moment the message is stored — before the turn
 *  is set up — so the gap between the two is a state this surface is in. */
let sendTurn: () => Promise<unknown>;

beforeEach(() => {
  listeners.clear();
  sendTurn = () => Promise.resolve(stored);
  vi.mocked(listen).mockImplementation(async (event, handler) => {
    listeners.set(String(event), handler);
    return () => {};
  });
  vi.mocked(invoke).mockImplementation((command: string) => {
    switch (command) {
      case 'discovery_get':
        return Promise.resolve(detail);
      case 'discovery_board':
        return Promise.resolve(board);
      case 'discovery_send_turn':
        return sendTurn();
      case 'workflow_list':
      case 'get_machines':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
});

async function openWorkspace() {
  const view = render(<DiscoveryView discoveryId="d-1" discoveryTitle="multi-client runner" />);
  await waitFor(() => expect(listeners.has('discovery_turn_status')).toBe(true));
  await view.findByPlaceholderText(/./);
  return view;
}

function status(payload: StatusPayload) {
  const handler = listeners.get('discovery_turn_status');
  if (!handler) throw new Error('the workspace never subscribed to discovery_turn_status');
  act(() => handler({ event: 'discovery_turn_status', id: 1, payload } as Event<unknown>));
}

describe('a turn that is still setting up', () => {
  it('holds the workspace pending and says so', async () => {
    const view = await openWorkspace();
    expect(view.queryByTestId('turn-activity')).toBeNull();

    status({ discovery_id: 'd-1', status: 'setting_up', reason: null });

    expect(view.getByTestId('turn-activity')).toBeInTheDocument();
    expect(view.getByTestId('turn-activity').textContent).toContain('Preparing the turn');
  });

  it('is still pending once the agent has the turn', async () => {
    const view = await openWorkspace();
    status({ discovery_id: 'd-1', status: 'setting_up', reason: null });
    status({ discovery_id: 'd-1', status: 'running', reason: null });

    // The phase alone changed, so nothing but the store's own coalesced wake
    // repaints the strip.
    await waitFor(() =>
      expect(view.getByTestId('turn-activity').textContent).toContain('Thinking'),
    );
  });

  it('lets the workspace go once the turn stops', async () => {
    const view = await openWorkspace();
    status({ discovery_id: 'd-1', status: 'setting_up', reason: null });
    status({ discovery_id: 'd-1', status: 'idle', reason: null });

    expect(view.queryByTestId('turn-activity')).toBeNull();
  });
});

describe('a turn that has been accepted but not yet set up', () => {
  async function typeAndSend(view: Awaited<ReturnType<typeof openWorkspace>>) {
    const composer = view.getByTestId('interview-composer');
    fireEvent.change(composer, { target: { value: 'what should this do?' } });
    fireEvent.click(view.getByTestId('interview-send'));
    return composer;
  }

  it('keeps the composer shut across the early return', async () => {
    let accept: ((message: DiscoveryMessage) => void) | null = null;
    sendTurn = () =>
      new Promise<DiscoveryMessage>((resolve) => {
        accept = resolve;
      });
    const view = await openWorkspace();
    const composer = await typeAndSend(view);
    expect(composer).toBeDisabled();

    await act(async () => {
      accept?.(stored);
    });
    // The message is stored and the promise is settled, but setup has not
    // started: a composer that reopened here would take a second turn that
    // kills the first agent's child.
    expect(composer).toBeDisabled();

    status({ discovery_id: 'd-1', status: 'setting_up', reason: null });
    expect(composer).toBeDisabled();
    status({ discovery_id: 'd-1', status: 'idle', reason: null });
    expect(composer).not.toBeDisabled();
  });

  it('shows a setup failure in the same banner a rejection would have used', async () => {
    const view = await openWorkspace();
    await typeAndSend(view);
    status({
      discovery_id: 'd-1',
      status: 'error',
      reason: "This project has no checkout on 'builder'",
    });

    expect(view.getByRole('alert').textContent).toContain('no checkout');
    expect(view.getByTestId('interview-composer')).not.toBeDisabled();
  });

  it('shows a refused turn and lets the user try again', async () => {
    sendTurn = () => Promise.reject('This discovery is already working');
    const view = await openWorkspace();
    const composer = await typeAndSend(view);

    await waitFor(() =>
      expect(view.getByRole('alert').textContent).toContain('already working'),
    );
    expect(composer).not.toBeDisabled();
  });
});
