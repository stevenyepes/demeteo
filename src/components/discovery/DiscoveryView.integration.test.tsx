// The two integration actions on DiscoveryWorkspaceHeader, wired through
// runAction to the typed wrappers in `lib/discovery.ts`. Both must go through
// `discovery_sync_base`/`discovery_publish_integration_mr` — never a raw
// `invoke()` — and a sync failure must surface verbatim in the same
// `actionError` banner every other action uses, with no second error surface.

import { fireEvent, render, waitFor } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Discovery, DiscoveryBoard, DiscoveryDetail, TicketView } from '../../types';
import { RoutedSelection } from '../../test/routedSelection';
import { DiscoveryView } from './DiscoveryView';
import { NavigationProvider } from '../../context';

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
  feature: { id: 'f-1', status: 'landed', mr_state: 'merged', mr_url: 'https://example.com/pr/1' },
};

const board: DiscoveryBoard = {
  tickets: [mergedTicket],
  progress: { blocked: 0, ready: 0, in_flight: 0, landed: 1, dropped: 0, live: 1 },
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
