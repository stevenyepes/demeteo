import { useCallback, useEffect, useState } from 'react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';

import { useTauriEvent } from '../../hooks/useTauriEvent';
import { useErrorBus } from '../../lib/errorBus';
import {
  abortSync,
  discardSyncResolution,
  getFeatureDivergence,
  getSyncSession,
  publishSyncResolution,
  reconcileSyncDivergence,
  continueSync,
  resolveSyncConflicts,
  syncFeature,
  type SyncResolverChoice,
} from '../../lib/featureSync';
import type { SyncIntent } from '../../lib/syncPanel';
import type { DivergenceReconcile, FeatureDivergence, SyncSessionView } from '../../types';

/** The two presses a divergence leaves open, named as the pane names them so
 *  `pending` labels the row that was actually pressed. */
type ReconcileIntent = Extract<SyncIntent, 'reconcile' | 'reset_onto_origin'>;

export interface SyncSession {
  session: SyncSessionView | null;
  /** What may be done about a sync that stopped on a divergence, measured
   *  against the two refs rather than remembered from the row. `null` on every
   *  other row, and on a branch nothing could read. */
  divergence: FeatureDivergence | null;
  /** Which action is in flight, or `null`. One value rather than a flag per
   *  intent: no two of these can overlap, and separate booleans let a render
   *  show two things happening at once. */
  pending: SyncIntent | null;
  refresh: () => Promise<void>;
  startSync: () => Promise<void>;
  resolve: (resolver: SyncResolverChoice) => Promise<void>;
  continueSync: () => Promise<void>;
  abort: () => Promise<void>;
  publish: () => Promise<void>;
  discard: () => Promise<void>;
  reconcile: (intent: ReconcileIntent) => Promise<void>;
}

/**
 * The feature's sync, read from the row that outlives this component.
 *
 * Every mutation ends by re-reading `sync_session_get` rather than folding its
 * own answer into state. The backend reconciles a session against the working
 * tree on the way out, so the row it hands back is the only one that has been
 * checked against git — and a conflict that existed only in a `useState` here
 * is the bug this hook was extracted to make unrepeatable: navigating away
 * unmounted it, and the conflicted worktree it described stayed on disk,
 * unnamed by anything in the UI, until the next sync force-removed it.
 *
 * Failures go to the error bus, not to `messageDialog`. A modal blocks the
 * whole window for a failure the pane can now render in place, and the pane is
 * where the user already is.
 */
export function useSyncSession(input: {
  featureId: string;
  /** The feature's run status. Re-read on every change: a run that just
   *  finished may have left a sync its own step started. */
  status: string;
  reload: () => void;
}): SyncSession {
  const { featureId, status, reload } = input;
  const { reportError } = useErrorBus();
  const [session, setSession] = useState<SyncSessionView | null>(null);
  const [divergence, setDivergence] = useState<FeatureDivergence | null>(null);
  const [pending, setPending] = useState<SyncIntent | null>(null);

  const read = useCallback(async () => {
    try {
      setSession(await getSyncSession(featureId));
    } catch (err) {
      reportError(err, { kind: 'internal' });
    }
  }, [featureId, reportError]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const fresh = await getSyncSession(featureId);
        if (!cancelled) setSession(fresh);
      } catch (err) {
        if (!cancelled) reportError(err, { kind: 'internal' });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [featureId, status, reportError]);

  /**
   * The backend's own announcement that this feature's session moved
   * (`DomainEvent::SyncStatusChanged`), which is the only thing that reaches a
   * pane nobody is pressing anything in.
   *
   * The effect above cannot stand in for it: the status it re-reads on is the
   * *run*'s, and the rollup that produces it excludes the out-of-band sync step
   * by design (`isOutOfBandStep`) — so a resolution running in the background
   * moves nothing this hook watches, and the pane kept rendering the
   * `resolving` it read on mount for the whole of one.
   *
   * The event's own status is deliberately not folded into state. Re-reading is
   * what reconciles the row against the worktree, and taking the status from
   * here would be the one reading that skipped that check.
   */
  useTauriEvent<{ feature_id: string; status: string }>('sync_status_changed', ({ feature_id }) => {
    if (feature_id === featureId) void read();
  });

  /**
   * The divergence, re-read on every row this hook lands, so a reconcile that
   * changed the answer does not leave the presses for the answer before it.
   *
   * A read that fails lands `null`, and `null` is rendered as the refusal that
   * offers nothing — the same reading the backend gives a `git cherry` it could
   * not run. It does not reach the error bus: nobody asked for this
   * measurement, and the pane already says what its absence means.
   */
  useEffect(() => {
    if (!stoppedOnDivergence(session)) {
      setDivergence(null);
      return;
    }
    let cancelled = false;
    getFeatureDivergence(featureId)
      .then((answer) => {
        if (!cancelled) setDivergence(answer);
      })
      .catch(() => {
        if (!cancelled) setDivergence(null);
      });
    return () => {
      cancelled = true;
    };
  }, [featureId, session]);

  const run = useCallback(
    async (intent: SyncIntent, call: () => Promise<unknown>, reloadRun: boolean) => {
      setPending(intent);
      try {
        await call();
      } catch (err) {
        reportError(err, { kind: 'internal' });
      } finally {
        await read();
        setPending(null);
      }
      if (reloadRun) reload();
    },
    [read, reload, reportError],
  );

  const refresh = useCallback(() => run('refresh', async () => {}, false), [run]);

  const startSync = useCallback(
    () => run('sync', () => syncFeature(featureId), true),
    [run, featureId],
  );

  const resolve = useCallback(
    (resolver: SyncResolverChoice) =>
      run('resolve', () => resolveSyncConflicts(featureId, resolver), true),
    [run, featureId],
  );

  const continueByHand = useCallback(
    () => run('continue', () => continueSync(featureId), true),
    [run, featureId],
  );

  const abort = useCallback(
    () => run('abort', () => abortSync(featureId), true),
    [run, featureId],
  );

  const publish = useCallback(
    () => run('publish', () => publishSyncResolution(featureId), true),
    [run, featureId],
  );

  /**
   * The confirmation says what actually happens — the branch moves back and the
   * sync is abandoned — because the tempting wording ("back to the conflict") is
   * a promise nothing here keeps: reproducing the conflict would mean re-running
   * the merge against an origin that has moved since.
   */
  const discard = useCallback(async () => {
    const ok = await confirmDialog(
      `Move ${session?.feature_branch ?? 'the branch'} back to where it was before the merge and abandon this sync? The conflict is not restored — sync again for a fresh one.`,
      { title: 'Discard the merge?', kind: 'warning', okLabel: 'Discard', cancelLabel: 'Keep' },
    );
    if (!ok) return;
    await run('discard', () => discardSyncResolution(featureId), true);
  }, [run, featureId, session?.feature_branch]);

  /**
   * The reset is confirmed and the merge is not, which is the line the backend
   * draws too: a merge can drop neither side, and the reset abandons commits
   * whose *changes* origin already carries — the commits themselves are what
   * the press gives up, and nothing in the history says whether they were meant
   * to survive.
   */
  const reconcile = useCallback(
    async (intent: ReconcileIntent) => {
      const branch = session?.feature_branch ?? 'the branch';
      if (intent === 'reset_onto_origin') {
        const ok = await confirmDialog(
          `Move ${branch} onto origin/${branch} and abandon the local commits? Origin already carries their changes; the commits themselves go.`,
          { title: 'Reset onto origin?', kind: 'warning', okLabel: 'Reset', cancelLabel: 'Keep' },
        );
        if (!ok) return;
      }
      const move: DivergenceReconcile =
        intent === 'reconcile' ? 'merge_origin' : 'reset_onto_origin';
      await run(intent, () => reconcileSyncDivergence(featureId, move), true);
    },
    [run, featureId, session?.feature_branch],
  );

  return {
    session,
    divergence,
    pending,
    refresh,
    startSync,
    resolve,
    continueSync: continueByHand,
    abort,
    publish,
    discard,
    reconcile,
  };
}

/** The one row a divergence reading is about. */
function stoppedOnDivergence(session: SyncSessionView | null): boolean {
  return session?.status === 'blocked' && session.blocked_stage === 'feature_diverged';
}
