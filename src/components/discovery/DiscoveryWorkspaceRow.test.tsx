/**
 * `DiscoveryWorkspaceRow` measures its own row (not the viewport) via
 * `useDiscoveryColumnLayout`, exactly as `useDiscoveryColumnLayout.test.ts`
 * drives it — jsdom lays nothing out, so every band change here comes from a
 * hand-triggered `ResizeObserverStub` tick against `offsetWidth`, never real
 * layout (`implementation-spec.md` §1 AC2).
 */
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { useMemo, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { NO_TURN } from '../../lib/discoveryActivity';
import { indexTickets } from '../../lib/ticketPresentation';
import type { Discovery, TicketView } from '../../types';
import { resizeObserverStubs } from '../../test/setup';
import { DiscoveryWorkspaceRow } from './DiscoveryWorkspaceRow';
import type { DiscoveryStreamStore } from './useDiscoveryStream';

vi.mocked(invoke).mockImplementation(((cmd: string) => {
  switch (cmd) {
    case 'list_agents':
      return Promise.resolve([]);
    default:
      return Promise.resolve(undefined);
  }
}) as unknown as typeof invoke);

const DISCOVERY: Discovery = {
  id: 'd-1',
  project_id: 'p-1',
  title: 'A discovery',
  status: 'open',
  machine_id: 'm-1',
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

const STORE: DiscoveryStreamStore = {
  subscribe: () => () => {},
  read: () => NO_TURN,
};

function ticket(seq: number): TicketView {
  return {
    ticket: {
      id: `t-${seq}`,
      discovery_id: DISCOVERY.id,
      seq,
      title: `Ticket ${seq}`,
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
    },
    standing: { id: `t-${seq}`, lane: 'ready', startable: true, blockers: [] },
    feature: null,
  };
}

const TICKETS = [ticket(1), ticket(2)];

/** Owns the same selection/editor state `DiscoveryView` does, so the row is
 *  driven exactly as its real caller drives it. */
function Harness({ pending = false }: { pending?: boolean } = {}) {
  const [selectedId, setSelectedId] = useState<string | null>(TICKETS[0].ticket.id);
  const [editingId, setEditingId] = useState<string | null>(null);
  const index = useMemo(() => indexTickets(TICKETS), []);
  const editing = editingId ? index.get(editingId) : undefined;
  const selected = selectedId ? index.get(selectedId) : undefined;

  return (
    <DiscoveryWorkspaceRow
      discovery={DISCOVERY}
      messages={[]}
      blocks={[]}
      machineLabel="local"
      pending={pending}
      store={STORE}
      onSend={() => {}}
      onRefresh={() => {}}
      tickets={TICKETS}
      index={index}
      progress={null}
      selectedId={selectedId}
      onSelect={setSelectedId}
      editing={editing}
      selected={selected}
      workflows={[]}
      workflowName={null}
      busy={false}
      machineId={DISCOVERY.machine_id}
      onEditorClose={() => setEditingId(null)}
      onInspectorClose={() => setSelectedId(null)}
      onEditorSaved={() => {}}
      onEditorStart={() => {}}
      onEditorForceStart={() => {}}
      onEditorDrop={() => {}}
      onInspectorStart={() => {}}
      onInspectorForceStart={() => {}}
      onInspectorEdit={() => selectedId && setEditingId(selectedId)}
      onInspectorOpenFeature={() => {}}
    />
  );
}

function setSize(el: HTMLElement, width: number, height = 800) {
  Object.defineProperty(el, 'offsetWidth', { configurable: true, value: width });
  Object.defineProperty(el, 'offsetHeight', { configurable: true, value: height });
}

function resizeTo(width: number, height = 800) {
  const row = screen.getByTestId('discovery-workspace-row');
  setSize(row, width, height);
  const observer = resizeObserverStubs.find((o) => o.observe.mock.calls.some(([t]) => t === row));
  if (!observer) throw new Error('no ResizeObserver was registered for the row');
  act(() => observer.trigger());
  return row;
}

describe('DiscoveryWorkspaceRow', () => {
  it('three-up (>=1280px): the inspector is an in-row sibling, no overlay', () => {
    render(<Harness />);
    const row = resizeTo(1400);

    const inspector = screen.getByTestId('ticket-verdict').closest('div');
    expect(inspector).not.toBeNull();
    expect(row).toContainElement(screen.getByTestId('ticket-verdict'));
    expect(screen.queryByLabelText('Ticket inspector')).not.toBeInTheDocument();
    expect(screen.queryByRole('radiogroup', { name: 'Workspace pane' })).not.toBeInTheDocument();
  });

  it('overlay-inspector (920-1279px): selecting/editing renders through TicketOverlayPanel while Interview/Tickets stay in-row', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    resizeTo(1000);

    // The inspector renders, but outside the row — through the portalled panel.
    const row = screen.getByTestId('discovery-workspace-row');
    const overlayBackdrop = screen.getByLabelText('Ticket inspector');
    expect(row.contains(overlayBackdrop)).toBe(false);
    expect(overlayBackdrop).toContainElement(screen.getByTestId('ticket-verdict'));

    // Interview and Tickets remain in-row, neither hidden.
    const composer = screen.getByTestId('interview-composer');
    expect(composer.closest('[aria-hidden="true"]')).toBeNull();
    expect(row.contains(composer)).toBe(true);
    const ticketView = document.querySelector('[aria-label="Ticket view"]');
    expect(ticketView?.closest('[aria-hidden="true"]')).toBeNull();
    expect(row.contains(ticketView)).toBe(true);

    // The overlay's backdrop must not intercept clicks/keystrokes aimed at the
    // row: drive an actual interaction at the composer and at a ticket node
    // while the inspector overlay is open, not just a containment check.
    await user.type(composer, 'a draft while the overlay is open');
    expect(composer).toHaveValue('a draft while the overlay is open');

    expect(screen.getByRole('heading', { name: 'Ticket 1' })).toBeInTheDocument();
    const [, secondNode] = screen.getAllByTestId('ticket-node');
    await user.click(secondNode);
    expect(screen.getByRole('heading', { name: 'Ticket 2' })).toBeInTheDocument();

    await user.click(screen.getByTestId('ticket-edit'));

    const editorBackdrop = screen.getByLabelText('Ticket editor');
    expect(row.contains(editorBackdrop)).toBe(false);
    expect(editorBackdrop).toContainElement(screen.getByTestId('ticket-editor'));
  });

  it.each([
    ['overlay-inspector', 1000],
    ['stacked', 600],
  ])('%s: Escape dismisses the inspector overlay, leaving no ticket selected', async (_name, width) => {
    render(<Harness />);
    resizeTo(width);

    expect(screen.getByLabelText('Ticket inspector')).toBeInTheDocument();

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });

    expect(screen.queryByLabelText('Ticket inspector')).not.toBeInTheDocument();
    expect(screen.queryByTestId('ticket-verdict')).not.toBeInTheDocument();
  });

  it('stacked (<920px): exactly one pane visible, toggling preserves the other pane’s state', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    resizeTo(600);

    expect(screen.getByRole('radiogroup', { name: 'Workspace pane' })).toBeInTheDocument();

    const composer = screen.getByTestId('interview-composer');
    expect(composer.closest('[aria-hidden="true"]')).toBeNull();
    const ticketView = document.querySelector('[aria-label="Ticket view"]');
    expect(ticketView?.closest('[aria-hidden="true"]')).not.toBeNull();

    await user.type(composer, 'a draft in progress');
    expect(composer).toHaveValue('a draft in progress');

    await user.click(screen.getByRole('radio', { name: 'Tickets' }));

    expect(composer.closest('[aria-hidden="true"]')).not.toBeNull();
    expect(ticketView?.closest('[aria-hidden="true"]')).toBeNull();
    // Not unmounted: the draft survived even while hidden.
    expect(composer).toHaveValue('a draft in progress');

    await user.click(screen.getByRole('radio', { name: 'Interview' }));

    expect(composer.closest('[aria-hidden="true"]')).toBeNull();
    expect(composer).toHaveValue('a draft in progress');
  });

  it('hiding the interview leaves the rail as the way back, and the draft intact behind it', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    resizeTo(1400);

    const composer = screen.getByTestId('interview-composer');
    await user.type(composer, 'a draft in progress');
    expect(screen.queryByTestId('interview-show')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('interview-hide'));

    expect(composer.closest('[aria-hidden="true"]')).not.toBeNull();
    expect(screen.getByTestId('interview-show')).toBeInTheDocument();
    // Not unmounted: the turn behind the rail is still the one being typed.
    expect(composer).toHaveValue('a draft in progress');

    await user.click(screen.getByTestId('interview-show'));

    expect(composer.closest('[aria-hidden="true"]')).toBeNull();
    expect(composer).toHaveValue('a draft in progress');
    expect(screen.queryByTestId('interview-show')).not.toBeInTheDocument();
  });

  it('a hidden interview hands its width to the inspector: 1000px seats it in-row instead of overlaying', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    resizeTo(1000);

    expect(screen.getByLabelText('Ticket inspector')).toBeInTheDocument();

    await user.click(screen.getByTestId('interview-hide'));

    const row = screen.getByTestId('discovery-workspace-row');
    expect(screen.queryByLabelText('Ticket inspector')).not.toBeInTheDocument();
    expect(row).toContainElement(screen.getByTestId('ticket-verdict'));
  });

  it('the rail pulses while a turn runs behind it', async () => {
    const user = userEvent.setup();
    render(<Harness pending />);
    resizeTo(1400);

    await user.click(screen.getByTestId('interview-hide'));

    expect(screen.getByTestId('interview-rail-pulse')).toBeInTheDocument();
  });

  it('stacked ignores a hide rather than emptying its own Interview pane, and honours it again on widening', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    resizeTo(1400);
    await user.click(screen.getByTestId('interview-hide'));

    // Below the graph's own minimum there is nothing left for a hide to buy,
    // so the pane toggle takes back over — with Interview on it.
    resizeTo(300);

    expect(screen.getByRole('radiogroup', { name: 'Workspace pane' })).toBeInTheDocument();
    expect(screen.queryByTestId('interview-show')).not.toBeInTheDocument();
    expect(screen.queryByTestId('interview-hide')).not.toBeInTheDocument();
    expect(screen.getByTestId('interview-composer').closest('[aria-hidden="true"]')).toBeNull();

    resizeTo(1400);

    expect(screen.getByTestId('interview-show')).toBeInTheDocument();
    expect(screen.getByTestId('interview-composer').closest('[aria-hidden="true"]')).not.toBeNull();
  });

  it('a resize crossing back above 920px during a stacked session shows both panes again', () => {
    render(<Harness />);
    resizeTo(600);
    expect(screen.getByRole('radiogroup', { name: 'Workspace pane' })).toBeInTheDocument();

    resizeTo(1000);

    expect(screen.queryByRole('radiogroup', { name: 'Workspace pane' })).not.toBeInTheDocument();
    const composer = screen.getByTestId('interview-composer');
    expect(composer.closest('[aria-hidden="true"]')).toBeNull();
    const ticketView = document.querySelector('[aria-label="Ticket view"]');
    expect(ticketView?.closest('[aria-hidden="true"]')).toBeNull();
  });
});
