/**
 * The middle of the review route: the card's refs have to survive the hop into
 * the editor view. Dropping them from the `EditorContext` here — with
 * `CodeEditorView` defaulting to the branch pair, as it does for every other
 * caller — silently opens the review on `defaultBranch..branch`, showing a diff
 * that omits the merge under review, with tsc and vitest both clean.
 */
import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AppView, RemoteRunMirror } from '../../types';

const getFeatureWorktree = vi.fn();
const getRemoteWorktree = vi.fn();
const openTerminalTab = vi.fn();

vi.mock('../../lib/featureDetail', () => ({
  getFeatureWorktree: (featureId: string) => getFeatureWorktree(featureId),
  getRemoteWorktree: (input: { machineId: string; runId: string }) => getRemoteWorktree(input),
}));

vi.mock('../../context', () => ({
  useTerminalPanel: () => ({ open: (req: unknown) => openTerminalTab(req) }),
}));

import { useWorktreeRouting } from './useWorktreeRouting';

describe('openDiffRange', () => {
  it('carries the ref pair and the tab into the editor view', async () => {
    getFeatureWorktree.mockResolvedValue({
      machine_id: 'local',
      worktree_path: '/repos/demeteo_wt_f-1',
      branch: 'feature/f-1',
      default_branch: 'master',
    });
    const views: AppView[] = [];
    const { result } = renderHook(() =>
      useWorktreeRouting({
        featureId: 'f-1',
        featureTitle: 'Add a metric strip',
        projectId: 'p-1',
        remoteRun: null,
        navigate: view => views.push(view),
      }),
    );

    await result.current.openDiffRange({ baseRef: 'aaaaaaa1111', headRef: 'c0ffeec2222' });

    await waitFor(() => expect(views).toHaveLength(1));
    expect(views[0]).toEqual({
      kind: 'editor',
      editorContext: {
        machineId: 'local',
        worktreePath: '/repos/demeteo_wt_f-1',
        branch: 'feature/f-1',
        defaultBranch: 'master',
        baseRef: 'aaaaaaa1111',
        headRef: 'c0ffeec2222',
        initialTab: 'changes',
      },
      featureId: 'f-1',
      featureTitle: 'Add a metric strip',
    });
  });
});

describe('handleOpenTerminalTab', () => {
  // A detached run's shadow feature still answers `feature_get_worktree` — with
  // a path under the local re-homed project, which is not where the runner's
  // code is. The shell must open on the runner's box, in the runner's path.
  it('opens a detached run in the runner worktree, not the shadow path', async () => {
    getFeatureWorktree.mockRejectedValue(new Error('shadow path must not be used'));
    getRemoteWorktree.mockResolvedValue({
      machine_id: 'box-1',
      worktree_path: '/srv/runner/work/demeteo_wt_f-1',
      branch: 'feature/f-1',
      default_branch: 'master',
    });
    const views: AppView[] = [];
    const { result } = renderHook(() =>
      useWorktreeRouting({
        featureId: 'f-1',
        featureTitle: 'Add a metric strip',
        projectId: 'p-1',
        remoteRun: { machine_id: 'box-1', run_id: 'r-1' } as RemoteRunMirror,
        navigate: view => views.push(view),
      }),
    );

    await result.current.handleOpenTerminalTab();

    expect(getRemoteWorktree).toHaveBeenCalledWith({ machineId: 'box-1', runId: 'r-1' });
    expect(openTerminalTab).toHaveBeenCalledWith(
      expect.objectContaining({
        machineId: 'box-1',
        workDir: '/srv/runner/work/demeteo_wt_f-1',
        workBranch: 'feature/f-1',
      }),
    );
    expect(views).toEqual([{ kind: 'terminals' }]);
  });
});
