import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';
import { createTerminalWorktree, listTerminalWorktrees } from '../lib/terminal';
import { NewTerminalMenu } from './NewTerminalMenu';

const open = vi.fn();
const useProject = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../hooks/useTerminalPanel', () => ({ useTerminalPanel: () => ({ open }) }));
vi.mock('../context/ProjectContext', () => ({ useProject: () => useProject() }));
vi.mock('../lib/terminal', () => ({
  createTerminalWorktree: vi.fn(),
  listTerminalWorktrees: vi.fn(),
}));

function mount() {
  return render(<NewTerminalMenu compact />);
}

async function openProjectLocations() {
  await userEvent.click(screen.getByTestId('new-terminal-caret'));
  await userEvent.click(screen.getByTestId('terminal-location-trigger'));
}

beforeEach(() => {
  open.mockReset();
  open.mockResolvedValue(undefined);
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation((command: string) => {
    if (command === 'get_machines' || command === 'get_agent_configs') return Promise.resolve([]);
    return Promise.resolve(undefined);
  });
  vi.mocked(listTerminalWorktrees).mockReset();
  vi.mocked(createTerminalWorktree).mockReset();
  useProject.mockReturnValue({
    state: {
      currentProjectId: 'project-1',
      projects: [{ id: 'project-1', name: 'Demo', status: 'idle', repos: 1, nodes: 0, spend: 0, tokens: 0 }],
      reposByProject: {
        'project-1': [{ id: 'repository-1', repo_path: '/repos/demo', provider_id: 'provider-1' }],
      },
    },
  });
});

describe('NewTerminalMenu terminal worktree locations', () => {
  it('requires an explicit project location before enabling shell or agent launch', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    mount();

    await openProjectLocations();

    expect(screen.getByRole('menuitem', { name: /new shell/i })).toBeDisabled();
    expect(screen.getByRole('menuitem', { name: /opencode/i })).toBeDisabled();
    await userEvent.click(screen.getByRole('menuitem', { name: /new shell/i }));
    expect(open).not.toHaveBeenCalled();
  });

  it('routes the primary local-shell action through the location chooser', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    mount();

    await userEvent.click(screen.getByTestId('new-terminal-trigger'));

    expect(open).not.toHaveBeenCalled();
    expect(await screen.findByTestId('new-terminal-dropdown')).toBeInTheDocument();
    expect(screen.getByTestId('terminal-location-trigger')).toHaveTextContent('Choose a location');
  });

  it('does not let Enter bypass an unselected project location', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    mount();

    await userEvent.click(screen.getByTestId('new-terminal-caret'));
    await userEvent.click(screen.getByTestId('new-terminal-search'));
    await userEvent.keyboard('{Enter}');

    expect(open).not.toHaveBeenCalled();
  });

  it('opens the current project primary checkout for the main-branch location', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    mount();

    await openProjectLocations();
    await userEvent.click(screen.getByTestId('terminal-location-main'));
    await userEvent.click(screen.getByText('New shell'));

    expect(open).toHaveBeenCalledWith(expect.objectContaining({
      projectId: 'project-1',
      repoPath: '/repos/demo',
      workDir: undefined,
      workBranch: null,
      forceNew: true,
    }));
  });

  it('opens a selected worktree shell with the exact backend path and branch', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([
      { path: '/repos/demo-ticket', branch: 'feature/ticket', isLocked: false },
    ]);
    mount();

    await openProjectLocations();
    await userEvent.click(await screen.findByTestId('terminal-location-worktree-/repos/demo-ticket'));
    await userEvent.click(screen.getByText('New shell'));

    expect(open).toHaveBeenCalledWith(expect.objectContaining({
      machineId: 'local',
      projectId: 'project-1',
      repoPath: '/repos/demo',
      workDir: '/repos/demo-ticket',
      workBranch: 'feature/ticket',
      forceNew: true,
      agentKind: null,
    }));
  });

  it('opens a created worktree with an agent using the returned target', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    vi.mocked(createTerminalWorktree).mockResolvedValue({
      path: '/repos/demo-new', branch: 'feature/new', isLocked: false,
    });
    mount();

    await openProjectLocations();
    await screen.findByText('No linked worktrees');
    await userEvent.type(screen.getByLabelText('Branch name'), 'feature/new');
    await userEvent.type(screen.getByLabelText('Worktree name'), 'demo-new');
    await userEvent.click(screen.getByTestId('terminal-location-create'));
    await userEvent.click(screen.getByText('OpenCode'));

    expect(open).toHaveBeenCalledWith(expect.objectContaining({
      workDir: '/repos/demo-new',
      workBranch: 'feature/new',
      forceNew: true,
      agentKind: 'opencode',
      launchCommand: 'opencode',
    }));
  });

  it('prevents a pending launch from opening duplicate sessions and formats failures', async () => {
    let rejectOpen: ((reason?: unknown) => void) | undefined;
    open.mockImplementation(() => new Promise((_, reject) => { rejectOpen = reject; }));
    // Local remains an ordinary machine-root target when the current project
    // belongs to a remote, so this keeps the split-button launch guard covered.
    useProject.mockReturnValue({
      state: {
        currentProjectId: 'project-1',
        projects: [{ id: 'project-1', name: 'Demo', status: 'idle', repos: 1, nodes: 0, spend: 0, tokens: 0, remote_host: 'remote-1' }],
        reposByProject: {
          'project-1': [{ id: 'repository-1', repo_path: '/repos/demo', provider_id: 'provider-1' }],
        },
      },
    });
    mount();

    const trigger = screen.getByTestId('new-terminal-trigger');
    await act(async () => {
      await userEvent.click(trigger);
      await userEvent.click(trigger);
    });
    expect(open).toHaveBeenCalledTimes(1);

    await act(async () => { rejectOpen?.({ kind: 'terminal', message: 'shell failed' }); });
    expect(await screen.findByTestId('new-terminal-error')).toHaveTextContent('shell failed');
  });
});
