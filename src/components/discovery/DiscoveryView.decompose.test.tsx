// A press of Decompose that fails is reported once. A pass that stopped is
// kept on the Discovery and its notice already carries the reason, so the
// action banner beside it would say the same thing twice; a refusal that was
// never kept has nothing else to say it.

import { fireEvent, render, waitFor } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { DecomposeProposal, Discovery, DiscoveryBoard, DiscoveryDetail } from '../../types';
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
  base_branch: null,
  integration_mr_url: null,
  integration_mr_state: null,
  attachments: [],
  total_cost: 0,
  tokens: 0,
  created_at: 0,
  updated_at: 0,
};

const board: DiscoveryBoard = {
  tickets: [],
  progress: { blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 0, live: 0 },
};

const STOP = 'Agent blocked: no output for 600s';

const stoppedPass: DecomposeProposal = {
  discovery_id: 'd-1',
  first_pass: true,
  tickets: [],
  changes: [],
  locked: [],
  refused: [],
  refusal: null,
  violations: [],
  stopped: STOP,
  cost_usd: 0,
  tokens: 0,
};

/** What `discovery_get` answers after the pass: the stored pass, or none. */
let afterPass: DecomposeProposal | null;
let decomposeError: string;
let passed = false;

beforeEach(() => {
  passed = false;
  vi.mocked(listen).mockImplementation(async () => () => {});
  vi.mocked(invoke).mockImplementation((command: string) => {
    switch (command) {
      case 'discovery_get': {
        const detail: DiscoveryDetail = {
          discovery,
          messages: [],
          pending_proposal: passed ? afterPass : null,
          turn_running: false,
        };
        return Promise.resolve(detail);
      }
      case 'discovery_board':
        return Promise.resolve(board);
      case 'discovery_decompose':
        passed = true;
        return Promise.reject(decomposeError);
      case 'workflow_list':
      case 'get_machines':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
});

async function pressDecompose() {
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
  fireEvent.click(await view.findByTestId('discovery-decompose'));
  return view;
}

describe('a decompose press that fails', () => {
  it('leaves a stopped pass to its notice alone', async () => {
    afterPass = stoppedPass;
    decomposeError = STOP;
    const view = await pressDecompose();

    await view.findByTestId('pending-proposal-decompose');
    expect(view.queryAllByText(new RegExp(STOP))).toHaveLength(1);
  });

  it('still reports a refusal that was never kept', async () => {
    afterPass = null;
    decomposeError = 'This discovery is already working';
    const view = await pressDecompose();

    await waitFor(() => expect(view.getByRole('alert').textContent).toContain('already working'));
    expect(view.queryByTestId('pending-proposal')).toBeNull();
  });
});
