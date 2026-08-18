/**
 * A conflict used to live in exactly one place the user could reach: the
 * `syncBanner` `useState` in this hook. Navigating away unmounted it, and the
 * conflicted worktree it described stayed on disk, unnamed by anything in the
 * UI, until the next sync force-removed it. These tests pin that the hook now
 * asks the backend instead of remembering.
 *
 * Every wrapper is stubbed per function and the module double throws on
 * anything it was not told to answer, because the global `invoke` mock in
 * `src/test/setup.ts` resolves `undefined` for every unstubbed command — the
 * TypeScript form of a double that can never fail (AGENTS.md §7).
 */
import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { SyncSessionView } from '../../types';

const getSyncSession = vi.fn<(featureId: string) => Promise<SyncSessionView | null>>();
const abortSync = vi.fn<(featureId: string) => Promise<SyncSessionView | null>>();

vi.mock('../../lib/featureSync', () => ({
  getSyncSession: (featureId: string) => getSyncSession(featureId),
  abortSync: (featureId: string) => abortSync(featureId),
  getFeature: async () => null,
  syncFeature: () => {
    throw new Error('syncFeature was not expected here');
  },
  resolveSyncConflicts: () => {
    throw new Error('resolveSyncConflicts was not expected here');
  },
  fetchMrState: () => {
    throw new Error('fetchMrState was not expected here');
  },
}));

vi.mock('../../lib/featureDetail', () => ({
  cleanupFeature: () => {
    throw new Error('cleanupFeature was not expected here');
  },
  publishMr: () => {
    throw new Error('publishMr was not expected here');
  },
}));

import { useFeatureMr } from './useFeatureMr';

const session = (over: Partial<SyncSessionView> = {}): SyncSessionView => ({
  feature_id: 'f-1',
  machine_id: 'local',
  repo_dir: '/repos/demeteo',
  feature_branch: 'feature/f-1',
  base_branch: 'master',
  status: 'conflicted',
  worktree_path: '/repos/demeteo_wt_sync_feature-f-1',
  head_before: 'aaaaaaa',
  merge_commit_sha: null,
  conflict_files: [{ path: 'src/lib.rs', kind: 'both modified' }],
  raw_error: 'CONFLICT (content): Merge conflict in src/lib.rs',
  user_may_intervene: true,
  attempts: 0,
  created_at: 0,
  updated_at: 0,
  ...over,
});

function mount() {
  return renderHook(() =>
    useFeatureMr({
      featureId: 'f-1',
      projectId: 'p-1',
      status: 'completed',
      reload: () => {},
      navigate: () => {},
    }),
  );
}

describe('useFeatureMr', () => {
  it('shows the persisted conflict on mount, files and git words intact', async () => {
    getSyncSession.mockResolvedValue(session());
    const { result } = mount();

    await waitFor(() => expect(result.current.syncBanner).not.toBeNull());
    expect(result.current.syncBanner).toEqual({
      status: 'conflict',
      conflict_files: [{ path: 'src/lib.rs', kind: 'both modified' }],
      raw_error: 'CONFLICT (content): Merge conflict in src/lib.rs',
    });
  });

  /**
   * The backend reconciles a session against the working tree before it
   * answers, so a status that is not `conflicted` is one there is nothing left
   * to act on — replaying it as a banner would put a dismissable notice about
   * last week's sync on every mount.
   */
  it('replays nothing for a session that is not a live conflict', async () => {
    for (const status of ['merged', 'aborted', 'up_to_date', 'blocked'] as const) {
      getSyncSession.mockResolvedValue(session({ status }));
      const { result, unmount } = mount();
      await waitFor(() => expect(getSyncSession).toHaveBeenCalled());
      expect(result.current.syncBanner).toBeNull();
      unmount();
      getSyncSession.mockClear();
    }
  });

  it('shows nothing for a feature that has never synced', async () => {
    getSyncSession.mockResolvedValue(null);
    const { result } = mount();

    await waitFor(() => expect(getSyncSession).toHaveBeenCalledWith('f-1'));
    expect(result.current.syncBanner).toBeNull();
  });

  it('clears the banner once the sync is abandoned', async () => {
    getSyncSession.mockResolvedValue(session());
    abortSync.mockResolvedValue(session({ status: 'aborted', worktree_path: null }));
    const { result } = mount();

    await waitFor(() => expect(result.current.syncBanner).not.toBeNull());
    await result.current.handleAbortSync();

    expect(abortSync).toHaveBeenCalledWith('f-1');
    await waitFor(() => expect(result.current.syncBanner).toBeNull());
  });
});

/**
 * Persisting the session created a footgun that could not exist before it: the
 * workflow's own `sync` step conflicts and resolves with no user involved, so a
 * hydrated banner can point its Abort and Resolve buttons at a worktree an
 * agent is mid-write in — abort deletes that directory, resolve puts a second
 * agent in it. The backend decides who owns a session; this pins that the hook
 * obeys rather than re-deriving it from `status`.
 */
it('leaves a conflict the run is still driving alone', async () => {
  getSyncSession.mockResolvedValue(session({ user_may_intervene: false }));

  const { result } = renderHook(() =>
    useFeatureMr({
      featureId: 'f-1',
      projectId: 'p-1',
      status: 'running',
      reload: () => {},
      navigate: () => {},
    }),
  );

  await waitFor(() => expect(getSyncSession).toHaveBeenCalledWith('f-1'));
  expect(result.current.syncBanner).toBeNull();
});
