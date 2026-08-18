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
const resolveSyncConflicts = vi.fn();
const publishSyncResolution = vi.fn<(featureId: string) => Promise<SyncSessionView | null>>();
const discardSyncResolution = vi.fn<(featureId: string) => Promise<SyncSessionView | null>>();

// `isAwaitingSyncReview` comes through real: it is the predicate under test in
// half of these, and a stubbed one would assert the stub.
vi.mock('../../lib/featureSync', async importOriginal => {
  const actual = await importOriginal<typeof import('../../lib/featureSync')>();
  return {
    isAwaitingSyncReview: actual.isAwaitingSyncReview,
    getSyncSession: (featureId: string) => getSyncSession(featureId),
    abortSync: (featureId: string) => abortSync(featureId),
    publishSyncResolution: (featureId: string) => publishSyncResolution(featureId),
    discardSyncResolution: (featureId: string) => discardSyncResolution(featureId),
    getFeature: async () => null,
    syncFeature: () => {
      throw new Error('syncFeature was not expected here');
    },
    resolveSyncConflicts: (featureId: string, files: string[]) =>
      resolveSyncConflicts(featureId, files),
    fetchMrState: () => {
      throw new Error('fetchMrState was not expected here');
    },
  };
});


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
  pushed_at: null,
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

/**
 * A resolution held for review has its own card, which says the thing the
 * banner cannot: that origin has not seen this yet. Two success notices for one
 * merge, one of which stops at "resolved", is how a user concludes it shipped —
 * so the banner is suppressed, and `reviewHeld` is what stops "Sync with main"
 * from writing over the row while it waits.
 */
it('suppresses the resolved banner while the resolution is still waiting', async () => {
  const held = session({ status: 'resolved', merge_commit_sha: 'c0ffeec', pushed_at: null });
  getSyncSession.mockResolvedValue(held);
  resolveSyncConflicts.mockResolvedValue({ status: 'resolved', merge_commit_sha: 'c0ffeec' });
  const { result } = mount();

  await waitFor(() => expect(getSyncSession).toHaveBeenCalledWith('f-1'));
  await result.current.handleResolveConflicts(['src/lib.rs']);

  await waitFor(() => expect(result.current.syncSession).toEqual(held));
  expect(result.current.syncBanner).toBeNull();
  expect(result.current.reviewHeld).toBe(true);
});

/** Once origin has it there is nothing left to review, so the ordinary success
 *  banner is the right and only notice. */
it('shows the resolved banner once the resolution is on origin', async () => {
  getSyncSession.mockResolvedValue(
    session({ status: 'resolved', merge_commit_sha: 'c0ffeec', pushed_at: 1700 }),
  );
  resolveSyncConflicts.mockResolvedValue({ status: 'resolved', merge_commit_sha: 'c0ffeec' });
  const { result } = mount();

  await waitFor(() => expect(getSyncSession).toHaveBeenCalledWith('f-1'));
  await result.current.handleResolveConflicts(['src/lib.rs']);

  await waitFor(() => expect(result.current.syncBanner).not.toBeNull());
  expect(result.current.reviewHeld).toBe(false);
});

/** A resolution a run still owns is nobody's to publish, and must not suppress
 *  the banner either: `user_may_intervene` is the backend's answer and the hook
 *  obeys it rather than re-deriving one from `status`. */
it('does not call a resolution held by a live run a review', async () => {
  getSyncSession.mockResolvedValue(
    session({
      status: 'resolved',
      merge_commit_sha: 'c0ffeec',
      pushed_at: null,
      user_may_intervene: false,
    }),
  );
  const { result } = mount();

  await waitFor(() => expect(getSyncSession).toHaveBeenCalledWith('f-1'));
  expect(result.current.reviewHeld).toBe(false);
});

/** Publishing answers with the row the backend reconciled on its way out, and
 *  the hook takes that rather than assuming the press worked. */
it('takes the published row from the publish call', async () => {
  const held = session({ status: 'resolved', merge_commit_sha: 'c0ffeec', pushed_at: null });
  getSyncSession.mockResolvedValue(held);
  publishSyncResolution.mockResolvedValue({ ...held, pushed_at: 1800 });
  const { result } = mount();

  await waitFor(() => expect(result.current.reviewHeld).toBe(true));
  await result.current.handlePublishSync();

  expect(publishSyncResolution).toHaveBeenCalledWith('f-1');
  await waitFor(() => expect(result.current.reviewHeld).toBe(false));
  expect(result.current.syncSession?.pushed_at).toBe(1800);
});

/** Discarding re-reads rather than trusting its own answer, because the row it
 *  returns has already been reconciled and the banner has to go with it. */
it('clears the banner and re-reads the row after a discard', async () => {
  const held = session({ status: 'resolved', merge_commit_sha: 'c0ffeec', pushed_at: null });
  const abandoned = session({ status: 'aborted', worktree_path: null, pushed_at: null });
  getSyncSession.mockResolvedValueOnce(held).mockResolvedValue(abandoned);
  discardSyncResolution.mockResolvedValue(abandoned);
  const { result } = mount();

  await waitFor(() => expect(result.current.reviewHeld).toBe(true));
  await result.current.handleDiscardSync();

  expect(discardSyncResolution).toHaveBeenCalledWith('f-1');
  await waitFor(() => expect(result.current.syncSession?.status).toBe('aborted'));
  expect(result.current.syncBanner).toBeNull();
  expect(result.current.reviewHeld).toBe(false);
});
