// The workspace-health panel's worktree list.
//
// `get_workspace_health` already excludes the primary checkout — the backend
// parser keeps only the linked worktrees (`domain::worktree_listing`). This tab
// used to drop a further entry on top of that, on the assumption that index 0
// was the main repo, so a repository with one linked worktree showed no badge
// and no rows at all.

import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { GeneralTab } from './GeneralTab';
import type { RepoHealthStatus, WorktreeInfo } from '../../lib/project';

const settings = vi.fn();

vi.mock('./ProjectSettingsContext', () => ({ useSettings: () => settings() }));
vi.mock('../../context', () => ({ useNavigation: () => ({ navigate: vi.fn() }) }));

function worktree(branch: string): WorktreeInfo {
  return { path: `/wt/${branch}`, branch, is_locked: false };
}

function mount(worktrees: WorktreeInfo[]) {
  const health: RepoHealthStatus = {
    repo_path: 'org/app',
    is_cloned: true,
    head_branch: 'main',
    worktrees,
    has_uncommitted: false,
    has_unpushed: false,
  };
  settings.mockReturnValue({
    projectName: 'demo',
    computeType: 'local',
    remoteHost: '',
    machines: [],
    selectedRepos: [],
    isTestingConnection: false,
    connectionStatus: null,
    showHealthPanel: true,
    healthExpanded: true,
    isLoadingHealth: false,
    healthError: '',
    healthData: [health],
    setProjectName: vi.fn(),
    setComputeType: vi.fn(),
    setRemoteHost: vi.fn(),
    setHealthExpanded: vi.fn(),
    handleTestConnection: vi.fn(),
    handleDeleteClick: vi.fn(),
    fetchAllReposFromProviders: vi.fn(),
    fetchWorkspaceHealth: vi.fn(),
    proceedWithReBootstrap: vi.fn(),
    setIsRepoModalOpen: vi.fn(),
    toggleRepo: vi.fn(),
  });
  return render(<GeneralTab />);
}

describe('GeneralTab workspace health', () => {
  it('lists every linked worktree the backend reported', () => {
    mount([worktree('terminal/one'), worktree('terminal/two')]);

    expect(screen.getByText('terminal/one')).toBeInTheDocument();
    expect(screen.getByText('terminal/two')).toBeInTheDocument();
    expect(screen.getByText('2 worktrees')).toBeInTheDocument();
  });

  it('shows a repository whose only linked worktree would have been dropped', () => {
    mount([worktree('terminal/only')]);

    expect(screen.getByText('terminal/only')).toBeInTheDocument();
    expect(screen.getByText('1 worktree')).toBeInTheDocument();
  });
});
