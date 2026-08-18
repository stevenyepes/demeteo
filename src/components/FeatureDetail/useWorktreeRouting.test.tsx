/**
 * The middle of the review route: the card's refs have to survive the hop into
 * the editor view. Dropping them from the `EditorContext` here — with
 * `CodeEditorView` defaulting to the branch pair, as it does for every other
 * caller — silently opens the review on `defaultBranch..branch`, showing a diff
 * that omits the merge under review, with tsc and vitest both clean.
 */
import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AppView } from '../../types';

const getFeatureWorktree = vi.fn();

vi.mock('../../lib/featureDetail', () => ({
  getFeatureWorktree: (featureId: string) => getFeatureWorktree(featureId),
  getRemoteWorktree: () => {
    throw new Error('no remote run in these tests');
  },
}));

vi.mock('../../context', () => ({
  useTerminalPanel: () => ({ open: () => {} }),
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
