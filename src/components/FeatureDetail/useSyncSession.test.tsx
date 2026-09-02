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
import { act, renderHook, waitFor } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FeatureDivergence, SyncSessionView } from '../../types';

const getSyncSession = vi.fn<(featureId: string) => Promise<SyncSessionView | null>>();
const getFeatureDivergence = vi.fn<(featureId: string) => Promise<FeatureDivergence | null>>();
const reconcileSyncDivergence = vi.fn();
const syncFeature = vi.fn();
const abortSync = vi.fn();
const resolveSyncConflicts = vi.fn();
const publishSyncResolution = vi.fn();
const discardSyncResolution = vi.fn();

vi.mock('../../lib/featureSync', () => ({
  getSyncSession: (featureId: string) => getSyncSession(featureId),
  getFeatureDivergence: (featureId: string) => getFeatureDivergence(featureId),
  reconcileSyncDivergence: (featureId: string, move: string) =>
    reconcileSyncDivergence(featureId, move),
  syncFeature: (featureId: string) => syncFeature(featureId),
  abortSync: (featureId: string) => abortSync(featureId),
  publishSyncResolution: (featureId: string) => publishSyncResolution(featureId),
  discardSyncResolution: (featureId: string) => discardSyncResolution(featureId),
  resolveSyncConflicts: (featureId: string, resolver: unknown) =>
    resolveSyncConflicts(featureId, resolver),
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
  blocked_stage: null,
  pushed_at: null,
  user_may_intervene: true,
  attempts: 1,
  created_at: 0,
  updated_at: 0,
  ...over,
});

const mount = (reload = () => {}) =>
  renderHook(() => useSyncSession({ featureId: 'f-1', status: 'completed', reload }));

const eventHandlers = new Map<string, (e: { payload: unknown }) => void>();

beforeEach(() => {
  eventHandlers.clear();
  vi.mocked(listen).mockImplementation((
    (event: string, handler: (e: { payload: unknown }) => void) => {
      eventHandlers.set(event, handler);
      return Promise.resolve(() => eventHandlers.delete(event));
    }
  ) as unknown as typeof listen);
});

async function announce(payload: { feature_id: string; status: string }): Promise<void> {
  const handler = eventHandlers.get('sync_status_changed');
  if (!handler) throw new Error('nothing subscribed to sync_status_changed');
  await act(async () => {
    handler({ payload });
  });
}

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

  it('carries the chosen resolver into the turn, and no file list', async () => {
    getSyncSession.mockResolvedValue(session());
    resolveSyncConflicts.mockResolvedValue(undefined);
    const { result } = mount();

    await waitFor(() => expect(result.current.session).not.toBeNull());
    await result.current.resolve({
      agentKind: 'codex',
      model: 'gpt-5-codex',
      effort: 'low',
    });

    expect(resolveSyncConflicts).toHaveBeenCalledWith('f-1', {
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

  /** The row says a divergence stopped the sync; what may be done about it is a
   *  property of two refs that both go on moving, so it is measured rather than
   *  remembered. */
  it('measures the divergence for a sync that stopped on one', async () => {
    const diverged = session({ status: 'blocked', blocked_stage: 'feature_diverged' });
    getSyncSession.mockResolvedValue(diverged);
    getFeatureDivergence.mockResolvedValue({ ahead: 2, behind: 3, next_move: 'reset_onto_origin' });
    const { result } = mount();

    await waitFor(() => expect(result.current.divergence?.next_move).toBe('reset_onto_origin'));
    expect(getFeatureDivergence).toHaveBeenCalledWith('f-1');
  });

  it('measures nothing for a row that did not stop on a divergence', async () => {
    getSyncSession.mockResolvedValue(session());
    const { result } = mount();

    await waitFor(() => expect(result.current.session).not.toBeNull());
    expect(getFeatureDivergence).not.toHaveBeenCalled();
    expect(result.current.divergence).toBeNull();
  });

  /** A measurement that could not be made is the same non-answer as a `git
   *  cherry` nobody could read, and the pane renders it as the refusal that
   *  offers no press — so it lands `null` rather than an error nobody asked
   *  for. */
  it('lands nothing when the divergence could not be read', async () => {
    getSyncSession.mockResolvedValue(session({ status: 'blocked', blocked_stage: 'feature_diverged' }));
    getFeatureDivergence.mockRejectedValue(new Error('fatal: bad revision'));
    const { result } = mount();

    await waitFor(() => expect(getFeatureDivergence).toHaveBeenCalled());
    expect(result.current.divergence).toBeNull();
  });

  /** The reset abandons commits. A confirm the user declined must leave the
   *  branch exactly where it was — the same rule the discard obeys. */
  it('writes nothing when the reset confirmation is declined', async () => {
    getSyncSession.mockResolvedValue(session({ status: 'blocked', blocked_stage: 'feature_diverged' }));
    getFeatureDivergence.mockResolvedValue({ ahead: 2, behind: 3, next_move: 'reset_onto_origin' });
    vi.mocked(confirmDialog).mockResolvedValueOnce(false);
    const { result } = mount();

    await waitFor(() => expect(result.current.session).not.toBeNull());
    await result.current.reconcile('reset_onto_origin');

    expect(reconcileSyncDivergence).not.toHaveBeenCalled();
  });

  /** The merge is the move that can drop neither side, so it asks nothing —
   *  and the intent the pane pressed decides the move on the wire. */
  it('merges origin in without asking, and names the move it sends', async () => {
    getSyncSession.mockResolvedValue(session({ status: 'blocked', blocked_stage: 'feature_diverged' }));
    getFeatureDivergence.mockResolvedValue({ ahead: 2, behind: 3, next_move: 'merge_origin' });
    reconcileSyncDivergence.mockResolvedValue(undefined);
    const { result } = mount();

    await waitFor(() => expect(result.current.session).not.toBeNull());
    await result.current.reconcile('reconcile');

    expect(confirmDialog).not.toHaveBeenCalled();
    expect(reconcileSyncDivergence).toHaveBeenCalledWith('f-1', 'merge_origin');
  });

  /** A resolution that finishes in the background moves nothing else this hook
   *  reads on: its other input is the run's status, and the rollup that
   *  produces it excludes the out-of-band sync step by design
   *  (`isOutOfBandStep`). So a pane left open across a resolution kept
   *  rendering the `resolving` it read on mount, beside a branch that had been
   *  merged and committed hours earlier. */
  it('re-reads the row when the backend announces a transition', async () => {
    getSyncSession
      .mockResolvedValueOnce(session({ status: 'resolving' }))
      .mockResolvedValue(session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222' }));
    const { result } = mount();
    await waitFor(() => expect(result.current.session?.status).toBe('resolving'));

    await announce({ feature_id: 'f-1', status: 'resolved' });

    await waitFor(() => expect(result.current.session?.status).toBe('resolved'));
  });

  /** One announcement reaches every open pane, and re-reading is four git
   *  invocations on the way out (`probe_worktree`) — over SSH for a remote
   *  session. */
  it('ignores a transition announced for another feature', async () => {
    getSyncSession.mockResolvedValue(session({ status: 'resolving' }));
    const { result } = mount();
    await waitFor(() => expect(result.current.session?.status).toBe('resolving'));
    getSyncSession.mockClear();

    await announce({ feature_id: 'f-other', status: 'resolved' });

    expect(getSyncSession).not.toHaveBeenCalled();
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
