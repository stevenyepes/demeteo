/**
 * A conflict used to live in exactly one place the user could reach: a
 * `useState` in the component that had watched it happen. Navigating away
 * unmounted it, and the conflicted worktree it described stayed on disk,
 * unnamed by anything in the UI, until the next sync force-removed it. These
 * pin that the hook asks the backend instead of remembering.
 *
 * Every wrapper is stubbed per function and the module double throws on
 * anything it was not told to answer, because the global `invoke` mock in
 * `src/test/setup.ts` resolves `undefined` for every unstubbed command — the
 * TypeScript form of a double that can never fail (AGENTS.md §7).
 */
import { renderHook, waitFor } from '@testing-library/react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { describe, expect, it, vi } from 'vitest';

import type { SyncSessionView } from '../../types';

const getSyncSession = vi.fn<(featureId: string) => Promise<SyncSessionView | null>>();
const syncFeature = vi.fn();
const abortSync = vi.fn();
const resolveSyncConflicts = vi.fn();
const publishSyncResolution = vi.fn();
const discardSyncResolution = vi.fn();

vi.mock('../../lib/featureSync', () => ({
  getSyncSession: (featureId: string) => getSyncSession(featureId),
  syncFeature: (featureId: string) => syncFeature(featureId),
  abortSync: (featureId: string) => abortSync(featureId),
  publishSyncResolution: (featureId: string) => publishSyncResolution(featureId),
  discardSyncResolution: (featureId: string) => discardSyncResolution(featureId),
  resolveSyncConflicts: (featureId: string, files: string[], resolver: unknown) =>
    resolveSyncConflicts(featureId, files, resolver),
}));

import { useSyncSession } from './useSyncSession';

const session = (over: Partial<SyncSessionView> = {}): SyncSessionView => ({
  feature_id: 'f-1',
  machine_id: 'local',
  repo_dir: '/repos/demeteo',
  feature_branch: 'feature/f-1',
  base_branch: 'origin/master',
  status: 'conflicted',
  worktree_path: '/repos/demeteo_wt_sync_feature-f-1',
  head_before: 'aaaaaaa1111',
  merge_commit_sha: null,
  conflict_files: [{ path: 'src/lib.rs', kind: 'both-modified' }],
  raw_error: 'CONFLICT (content): Merge conflict in src/lib.rs',
  pushed_at: null,
  user_may_intervene: true,
  attempts: 1,
  created_at: 0,
  updated_at: 0,
  ...over,
});

const mount = (reload = () => {}) =>
  renderHook(() => useSyncSession({ featureId: 'f-1', status: 'completed', reload }));

describe('useSyncSession', () => {
  it('reads the persisted session on mount, files and git words intact', async () => {
    getSyncSession.mockResolvedValue(session());
    const { result } = mount();

    await waitFor(() => expect(result.current.session).not.toBeNull());
    expect(result.current.session?.conflict_files).toEqual([
      { path: 'src/lib.rs', kind: 'both-modified' },
    ]);
    expect(result.current.session?.raw_error).toBe(
      'CONFLICT (content): Merge conflict in src/lib.rs',
    );
  });

  /** The whole reason the row exists: the conflict outlives the component that
   *  watched it happen, so a remount has to find it again rather than open on
   *  an empty pane beside a worktree with `MERGE_HEAD` set. */
  it('finds the conflict again after a remount', async () => {
    getSyncSession.mockResolvedValue(session());
    const first = mount();
    await waitFor(() => expect(first.result.current.session?.status).toBe('conflicted'));
    first.unmount();

    const second = mount();
    await waitFor(() => expect(second.result.current.session?.status).toBe('conflicted'));
    expect(getSyncSession).toHaveBeenCalledTimes(2);
  });

  it('shows nothing for a feature that has never synced', async () => {
    getSyncSession.mockResolvedValue(null);
    const { result } = mount();

    await waitFor(() => expect(getSyncSession).toHaveBeenCalledWith('f-1'));
    expect(result.current.session).toBeNull();
  });

  /** Every mutation ends by re-reading, because the backend reconciles a
   *  session against the working tree on the way out — the row it hands back is
   *  the only one that has been checked against git. */
  it('re-reads the row after an abort rather than trusting the press', async () => {
    getSyncSession.mockResolvedValueOnce(session()).mockResolvedValue(
      session({ status: 'aborted', worktree_path: null }),
    );
    abortSync.mockResolvedValue(undefined);
    const { result } = mount();

    await waitFor(() => expect(result.current.session?.status).toBe('conflicted'));
    await result.current.abort();

    expect(abortSync).toHaveBeenCalledWith('f-1');
    await waitFor(() => expect(result.current.session?.status).toBe('aborted'));
  });

  it('re-reads the row after a publish', async () => {
    const held = session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222' });
    getSyncSession.mockResolvedValueOnce(held).mockResolvedValue({ ...held, pushed_at: 1800 });
    publishSyncResolution.mockResolvedValue(undefined);
    const { result } = mount();

    await waitFor(() => expect(result.current.session?.pushed_at).toBeNull());
    await result.current.publish();

    expect(publishSyncResolution).toHaveBeenCalledWith('f-1');
    await waitFor(() => expect(result.current.session?.pushed_at).toBe(1800));
  });

  it('carries the chosen resolver and the file list into the turn', async () => {
    getSyncSession.mockResolvedValue(session());
    resolveSyncConflicts.mockResolvedValue(undefined);
    const { result } = mount();

    await waitFor(() => expect(result.current.session).not.toBeNull());
    await result.current.resolve(['src/lib.rs'], {
      agentKind: 'codex',
      model: 'gpt-5-codex',
      effort: 'low',
    });

    expect(resolveSyncConflicts).toHaveBeenCalledWith('f-1', ['src/lib.rs'], {
      agentKind: 'codex',
      model: 'gpt-5-codex',
      effort: 'low',
    });
  });

  /** Discard moves a branch. A confirm the user declined must leave the row
   *  exactly where it was. */
  it('writes nothing when the discard confirmation is declined', async () => {
    getSyncSession.mockResolvedValue(session({ status: 'resolved', merge_commit_sha: 'c0ffeec' }));
    vi.mocked(confirmDialog).mockResolvedValueOnce(false);
    const { result } = mount();

    await waitFor(() => expect(result.current.session).not.toBeNull());
    await result.current.discard();

    expect(discardSyncResolution).not.toHaveBeenCalled();
  });

  it('reloads the run after a sync, so the timeline sees the merge', async () => {
    getSyncSession.mockResolvedValue(session({ status: 'merged' }));
    syncFeature.mockResolvedValue(undefined);
    const reload = vi.fn();
    const { result } = mount(reload);

    await waitFor(() => expect(result.current.session).not.toBeNull());
    await result.current.startSync();

    expect(syncFeature).toHaveBeenCalledWith('f-1');
    expect(reload).toHaveBeenCalled();
  });
});
