import { useCallback, useEffect, useState } from 'react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';

import { useErrorBus } from '../../lib/errorBus';
import {
  abortSync,
  discardSyncResolution,
  getSyncSession,
  publishSyncResolution,
  resolveSyncConflicts,
  syncFeature,
  type SyncResolverChoice,
} from '../../lib/featureSync';
import type { SyncIntent } from '../../lib/syncPanel';
import type { SyncSessionView } from '../../types';

export interface SyncSession {
  session: SyncSessionView | null;
  /** Which action is in flight, or `null`. One value rather than a flag per
   *  intent: no two of these can overlap, and separate booleans let a render
   *  show two things happening at once. */
  pending: SyncIntent | null;
  refresh: () => Promise<void>;
  startSync: () => Promise<void>;
  resolve: (files: string[], resolver: SyncResolverChoice) => Promise<void>;
  abort: () => Promise<void>;
  publish: () => Promise<void>;
  discard: () => Promise<void>;
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
    (files: string[], resolver: SyncResolverChoice) =>
      run('resolve', () => resolveSyncConflicts(featureId, files, resolver), true),
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

  return { session, pending, refresh, startSync, resolve, abort, publish, discard };
}
