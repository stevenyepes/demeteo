// DiscoveryWorkspaceHeader — the base-branch control. `baseBranchLock` is unit
// tested on its own; these pin the wiring the lib cannot see: that the one
// `lock` result drives both the disabled "Change" button and the banner, and
// that a refused `setDiscoveryBase` stays inside the still-open editor rather
// than closing it or reaching `onBaseChanged`.

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const setDiscoveryBase = vi.fn();
const getRepositoriesForProject = vi.fn();
const listTerminalBranches = vi.fn();

vi.mock('../../lib/discovery', () => ({
  setDiscoveryBase: (...args: unknown[]) => setDiscoveryBase(...args),
}));

vi.mock('../../lib/project', () => ({
  getRepositoriesForProject: (...args: unknown[]) => getRepositoriesForProject(...args),
}));

vi.mock('../../lib/terminal', () => ({
  listTerminalBranches: (...args: unknown[]) => listTerminalBranches(...args),
}));

vi.mock('../ui/BackButton', () => ({
  BackButton: () => null,
}));

import { DiscoveryWorkspaceHeader } from './DiscoveryWorkspaceHeader';
import type { Discovery, DiscoveryBoard, Ticket, TicketView } from '../../types';

function discovery(overrides: Partial<Discovery> = {}): Discovery {
  return {
    id: 'disc-1',
    project_id: 'proj-1',
    title: 'A discovery',
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
    ...overrides,
  };
}

function ticket(seq: number, state: Ticket['state']): TicketView {
  return {
    ticket: {
      id: `t${seq}`,
      discovery_id: 'disc-1',
      seq,
      title: `ticket ${seq}`,
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
      state,
      drop_reason: null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: null,
      created_at: 0,
      updated_at: 0,
    },
    standing: { id: `t${seq}`, lane: 'ready', startable: false, blockers: [] },
    feature: null,
  };
}

function board(...tickets: TicketView[]): DiscoveryBoard {
  return {
    tickets,
    progress: { blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 0, live: tickets.length },
  };
}

function mount(props: { discovery?: Discovery; board?: DiscoveryBoard | null } = {}) {
  const onBaseChanged = vi.fn();
  render(
    <DiscoveryWorkspaceHeader
      discovery={props.discovery ?? discovery()}
      board={props.board === undefined ? board() : props.board}
      turnCount={0}
      turnRunning={false}
      onToggleOpen={() => {}}
      onDecompose={() => {}}
      decomposing={false}
      busy={false}
      projectId="proj-1"
      onBaseChanged={onBaseChanged}
    />,
  );
  return { onBaseChanged };
}

beforeEach(() => {
  setDiscoveryBase.mockReset();
  getRepositoriesForProject.mockReset();
  listTerminalBranches.mockReset();
  getRepositoriesForProject.mockResolvedValue([{ id: 'repo-1', repo_path: '/x', provider_id: 'git' }]);
  listTerminalBranches.mockResolvedValue({ defaultBranch: 'main', branches: [] });
});

afterEach(cleanup);

describe('DiscoveryWorkspaceHeader base branch', () => {
  it('shows the generic default label and an enabled Change while every ticket is unstarted', () => {
    mount({ board: board(ticket(1, 'unstarted'), ticket(2, 'unstarted')) });

    expect(screen.getByText("Starts from the project's default branch")).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Change' }) as HTMLButtonElement).disabled).toBe(
      false,
    );
    expect(screen.queryByTestId('discovery-base-locked')).toBeNull();
  });

  it('disables Change and shows the same reason once a ticket has left unstarted', () => {
    mount({ board: board(ticket(1, 'unstarted'), ticket(2, 'dropped')) });

    expect((screen.getByRole('button', { name: 'Change' }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(screen.getByTestId('discovery-base-locked').textContent).toContain('Ticket DSC-2');
  });

  it('renders a set base branch verbatim', () => {
    mount({ discovery: discovery({ base_branch: 'feat/integration' }) });

    expect(screen.getByText('feat/integration')).toBeTruthy();
  });

  it('saves a named branch, hands the returned Discovery to onBaseChanged, and closes the editor', async () => {
    const user = userEvent.setup();
    const updated = discovery({ base_branch: 'feat/x' });
    setDiscoveryBase.mockResolvedValue(updated);
    const { onBaseChanged } = mount();

    await user.click(screen.getByRole('button', { name: 'Change' }));
    await user.click(screen.getByRole('radio', { name: 'Named branch' }));
    await user.type(screen.getByLabelText('Base branch name'), 'feat/x');
    await user.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(onBaseChanged).toHaveBeenCalledWith(updated));
    expect(setDiscoveryBase).toHaveBeenCalledWith('disc-1', 'feat/x');
    expect(screen.queryByRole('radiogroup', { name: 'Base branch' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Change' })).toBeTruthy();
  });

  it('keeps the editor open with the error inline when setDiscoveryBase is refused', async () => {
    const user = userEvent.setup();
    setDiscoveryBase.mockRejectedValue(new Error('branch not found'));
    const { onBaseChanged } = mount();

    await user.click(screen.getByRole('button', { name: 'Change' }));
    await user.click(screen.getByRole('radio', { name: 'Named branch' }));
    await user.type(screen.getByLabelText('Base branch name'), 'feat/missing');
    await user.click(screen.getByRole('button', { name: 'Save' }));

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('branch not found');
    expect(onBaseChanged).not.toHaveBeenCalled();
    expect(screen.getByRole('radiogroup', { name: 'Base branch' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Save' })).toBeTruthy();
  });

  it('cancel closes the editor without calling setDiscoveryBase', async () => {
    const user = userEvent.setup();
    const { onBaseChanged } = mount();

    await user.click(screen.getByRole('button', { name: 'Change' }));
    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(setDiscoveryBase).not.toHaveBeenCalled();
    expect(onBaseChanged).not.toHaveBeenCalled();
    expect(screen.queryByRole('radiogroup', { name: 'Base branch' })).toBeNull();
  });
});
