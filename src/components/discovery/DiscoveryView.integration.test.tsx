// The two integration actions on DiscoveryWorkspaceHeader, wired through
// runAction to the typed wrappers in `lib/discovery.ts`. Both must go through
// `discovery_sync_base`/`discovery_publish_integration_mr` — never a raw
// `invoke()` — and a sync failure must surface verbatim in the same
// `actionError` banner every other action uses, with no second error surface.

import { act, fireEvent, render, waitFor, within } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Discovery, DiscoveryBoard, DiscoveryDetail, Feature, TicketView } from '../../types';
import { RoutedSelection } from '../../test/routedSelection';
import { DiscoveryView } from './DiscoveryView';
import { NavigationProvider, useNavigation } from '../../context';
import { ErrorToast } from '../ErrorToast';
import { ErrorBusProvider, useErrorBus } from '../../lib/errorBus';

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
  base_branch: 'integration/multi-client-runner',
  integration_mr_url: null,
  integration_mr_state: null,
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

const mergedTicket: TicketView = {
  ticket: {
    id: 't-1',
    discovery_id: 'd-1',
    seq: 1,
    title: 'Ticket 1',
    description: '',
    acceptance: [],
    files: [],
    blocked_by: [],
    test_command: null,
    workflow_id: null,
    agent_kind: null,
    model: null,
    effort: null,
    machine_id: null,
    attachments: [],
    state: 'started',
    drop_reason: null,
    force_start_reason: null,
    force_started_at: null,
    feature_id: 'f-1',
    created_at: 0,
    updated_at: 0,
  },
  standing: { id: 't-1', lane: 'landed', startable: false, blockers: [] },
  placement: { placement: { kind: 'local' }, inherited: true },
  feature: { id: 'f-1', status: 'landed', mr_state: 'merged', mr_url: 'https://example.com/pr/1', placement: null, remote: null },
};

const board: DiscoveryBoard = {
  tickets: [mergedTicket],
  progress: { blocked: 0, ready: 0, in_flight: 0, landed: 1, dropped: 0, live: 1 },
  discovery_default: { kind: 'local' },
  local_host: 'local',
};

beforeEach(() => {
  vi.mocked(listen).mockImplementation(async () => () => {});
});

function mockInvoke(overrides: Record<string, () => Promise<unknown>>) {
  vi.mocked(invoke).mockImplementation((command: string) => {
    if (command in overrides) return overrides[command]();
    switch (command) {
      case 'discovery_get':
        return Promise.resolve(detail);
      case 'discovery_board':
        return Promise.resolve(board);
      case 'workflow_list':
      case 'get_machines':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
}

async function openWorkspace() {
  const view = render(
    <RoutedSelection>
      {(selectedTicketId, onSelectTicket) => (
        <DiscoveryView
          discoveryId="d-1"
          discoveryTitle="multi-client runner"
          selectedTicketId={selectedTicketId}
          onSelectTicket={onSelectTicket}
        />
      )}
    </RoutedSelection>,
    { wrapper: NavigationProvider },
  );
  await view.findByTestId('discovery-update-base');
  return view;
}

describe('the integration controls', () => {
  it('syncs the base branch through discovery_sync_base', async () => {
    const syncBase = vi.fn(() => Promise.resolve({ updated: true }));
    mockInvoke({ discovery_sync_base: syncBase });
    const view = await openWorkspace();

    fireEvent.click(view.getByTestId('discovery-update-base'));

    await waitFor(() => expect(syncBase).toHaveBeenCalledTimes(1));
    expect(invoke).not.toHaveBeenCalledWith(
      expect.stringContaining('publish'),
      expect.anything(),
    );
  });

  it('surfaces a sync failure verbatim in the existing actionError banner', async () => {
    const failureDetail = 'conflict in src/lib/discovery.ts, src/lib/discoveryIntegration.ts';
    mockInvoke({ discovery_sync_base: () => Promise.reject(failureDetail) });
    const view = await openWorkspace();

    fireEvent.click(view.getByTestId('discovery-update-base'));

    const alert = await view.findByRole('alert');
    expect(alert.textContent).toContain(failureDetail);
    expect(view.queryAllByRole('alert')).toHaveLength(1);
  });

  it('publishes the integration PR through discovery_publish_integration_mr', async () => {
    const publish = vi.fn(() =>
      Promise.resolve({ id: 'f-1', mr_url: 'https://example.com/pr/2', mr_state: 'open' }),
    );
    mockInvoke({ discovery_publish_integration_mr: publish });
    const view = await openWorkspace();

    fireEvent.click(view.getByTestId('discovery-publish-integration'));

    await waitFor(() => expect(publish).toHaveBeenCalledTimes(1));
  });
});

// Starting a ticket goes through `launch_run`, so a detached start can be
// refused by the runner gate. That refusal has a remedy the banner cannot
// offer, and a detached start is slow enough to be clicked twice.

const readyTicket: TicketView = {
  ...mergedTicket,
  ticket: { ...mergedTicket.ticket, state: 'unstarted', feature_id: null, machine_id: 'm-box' },
  standing: { id: 't-1', lane: 'ready', startable: true, blockers: [] },
  placement: { placement: { kind: 'detached', machine_id: 'm-box' }, inherited: false },
  feature: null,
};

const readyBoard: DiscoveryBoard = {
  tickets: [readyTicket],
  progress: { blocked: 0, ready: 1, in_flight: 0, landed: 0, dropped: 0, live: 1 },
  discovery_default: { kind: 'local' },
  local_host: 'local',
};

const refusalMessage = 'The runner on box is 1.0.0; this Demeteo is 1.1.0. Upgrade the runner.';
const refusal = {
  kind: 'runner_incompatible',
  message: refusalMessage,
  compatibility: {
    verdict: 'runner_behind',
    runner: '1.0.0',
    runner_channel: 'stable',
    app: '1.1.0',
    app_channel: 'stable',
  },
};

function CurrentView() {
  const { view } = useNavigation();
  return <output data-testid="current-view">{view.kind}</output>;
}

function ClearToasts() {
  const { clear } = useErrorBus();
  return <button type="button" data-testid="clear-toasts" onClick={clear} />;
}

async function openReadyTicket(start: () => Promise<unknown>) {
  mockInvoke({ discovery_board: () => Promise.resolve(readyBoard), ticket_start: start });
  const view = render(
    <ErrorBusProvider>
      <NavigationProvider>
        <RoutedSelection>
          {(selectedTicketId, onSelectTicket) => (
            <DiscoveryView
              discoveryId="d-1"
              discoveryTitle="multi-client runner"
              selectedTicketId={selectedTicketId}
              onSelectTicket={onSelectTicket}
            />
          )}
        </RoutedSelection>
        <ErrorToast />
        <CurrentView />
        <ClearToasts />
      </NavigationProvider>
    </ErrorBusProvider>,
  );
  const button = await view.findByRole('button', { name: 'Start ticket' });
  return { view, button };
}

describe('starting a ticket', () => {
  it('renders a runner_incompatible refusal as a toast that routes to machine settings', async () => {
    const start = vi.fn(() => Promise.reject(refusal));
    const { view, button } = await openReadyTicket(start);

    fireEvent.click(button);

    const toast = await view.findByTestId('error-toast-runner_incompatible');
    expect(toast.textContent).toContain(refusalMessage);
    expect(view.queryByText(refusalMessage, { selector: 'p' })).toBeNull();
    expect(invoke).toHaveBeenCalledWith('ticket_start', { ticketId: 't-1', machineId: null });

    fireEvent.click(within(toast).getByRole('button', { name: 'Open machine settings' }));

    expect(view.getByTestId('current-view').textContent).toBe('settings');
    act(() => fireEvent.click(view.getByTestId('clear-toasts')));
  });

  it('keeps any other start failure in the action banner', async () => {
    const { view, button } = await openReadyTicket(() => Promise.reject('ticket t-1 is locked'));

    fireEvent.click(button);

    const alert = await view.findByText('ticket t-1 is locked');
    expect(alert.getAttribute('role')).toBe('alert');
    expect(view.queryByTestId('error-toast-runner_incompatible')).toBeNull();
  });

  it('disables Start while the start is pending, so it cannot be submitted twice', async () => {
    let resolve: (feature: Feature) => void = () => {};
    const start = vi.fn(
      () =>
        new Promise<Feature>((settle) => {
          resolve = settle;
        }),
    );
    const { button } = await openReadyTicket(start);

    fireEvent.click(button);
    fireEvent.click(button);

    expect(button).toBeDisabled();
    expect(start).toHaveBeenCalledTimes(1);
    await act(async () => resolve({ id: 'f-1' } as Feature));
    await waitFor(() => expect(button).not.toBeDisabled());
  });

  it('starts a ticket whose credentials were parked, and says so in a notice that offers Runs', async () => {
    const parked = { id: 'f-1', title: 'the ticket', credentials_parked: 'unscripted rpc inject_credentials' };
    const { view, button } = await openReadyTicket(() => Promise.resolve(parked));

    fireEvent.click(button);

    const toast = await view.findByTestId('error-toast-provider');
    expect(toast.textContent).toContain('unscripted rpc inject_credentials');
    expect(view.queryByText(/inject_credentials/, { selector: 'p' })).toBeNull();

    fireEvent.click(within(toast).getByRole('button', { name: 'Open runs' }));

    expect(view.getByTestId('current-view').textContent).toBe('remote-inbox');
    act(() => fireEvent.click(view.getByTestId('clear-toasts')));
  });

  it('posts no notice for a clean start', async () => {
    const start = vi.fn(() => Promise.resolve({ id: 'f-1', title: 'the ticket' }));
    const { view, button } = await openReadyTicket(start);

    fireEvent.click(button);

    await waitFor(() => expect(start).toHaveBeenCalledOnce());
    await waitFor(() => expect(button).not.toBeDisabled());
    expect(view.queryByRole('alert')).toBeNull();
  });
});
