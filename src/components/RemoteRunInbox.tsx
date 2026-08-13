import React, { useCallback, useEffect, useMemo, useState } from 'react';
import {
  RefreshCw,
  CheckCircle2,
  XCircle,
  PauseCircle,
  KeyRound,
  Loader,
  WifiOff,
  ExternalLink,
  Ban,
  Inbox,
  ListTree,
} from 'lucide-react';
import type { Machine, RemoteRunMirror } from '../types';
import { Chip } from './ui/Chip';
import { RemoteGateActions, ReinjectCredentials } from './RemoteRunActions';
import { TONE_BORDER_L, TONE_TEXT, type RunStatusTone } from '../lib/runStatus';
import { relativeTime } from '../lib/utils';
import { bucketFor, type Bucket } from '../lib/remoteRunBuckets';
import { useNavigation } from '../context';
import { formatError } from '../lib/errors';
import { listMachines } from '../lib/machines';
import {
  cancelRemoteRun,
  listMirroredRuns,
  reconcileRuns,
  remoteRunDiffUrl,
} from '../lib/remoteRuns';

/**
 * Runs — the cross-machine attention hub (docs/REMOTE_EXECUTION.md
 * M6.2, design §8). Groups every mirrored remote run under the buckets
 * `lib/remoteRunBuckets.ts` defines, and renders each group.
 *
 * This page triages; the run itself lives in `FeatureDetail` — every
 * row deep-links there (the eager shadow feature guarantees an id from
 * submit time), where the step timeline and the Activity event feed
 * render together.
 */

const BUCKET_ORDER: Bucket[] = [
  'parked',
  'needs_credentials',
  'failed',
  'pr_ready',
  'running',
  'unreachable',
  'cancelled',
];

/** Per-bucket label + icon + tone. Tones are the shared status
 *  vocabulary's (`lib/runStatus.ts`, F27): each bucket carries the tone
 *  of the statuses it groups, so the inbox speaks the same color
 *  language as every other run surface. */
const BUCKET_META: Record<
  Bucket,
  { label: string; icon: React.ComponentType<{ className?: string }>; tone: RunStatusTone }
> = {
  pr_ready: { label: 'PR ready', icon: CheckCircle2, tone: 'emerald' },
  failed: { label: 'Failed', icon: XCircle, tone: 'ruby' },
  parked: { label: 'Parked — needs you', icon: PauseCircle, tone: 'amber' },
  needs_credentials: { label: 'Needs credentials', icon: KeyRound, tone: 'amber' },
  running: { label: 'Running', icon: Loader, tone: 'cyan' },
  unreachable: { label: 'Unreachable', icon: WifiOff, tone: 'slate' },
  cancelled: { label: 'Cancelled', icon: Ban, tone: 'slate' },
};

/**
 * "View branch diff" (docs/REMOTE_EXECUTION.md M6.2 follow-up): a
 * run that pushed its feature branch but has no PR yet (failed/
 * cancelled/parked) still produced code worth looking at. Resolves the
 * compare/tree URL lazily per row rather than eagerly for every run in
 * the list, and hides itself entirely if the backend can't resolve a
 * repo/provider for it (a missing deep link, not a broken one).
 */
const DiffLinkButton: React.FC<{ run: RemoteRunMirror }> = ({ run }) => {
  const [url, setUrl] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    if (!run.project_id || !run.pushed_branch) {
      setUrl(null);
      return;
    }
    let cancelled = false;
    remoteRunDiffUrl(run.project_id, run.pushed_branch)
      .then((u) => { if (!cancelled) setUrl(u); })
      .catch(() => { if (!cancelled) setUrl(null); });
    return () => { cancelled = true; };
  }, [run.project_id, run.pushed_branch]);

  if (!url) return null;
  return (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5"
      title={`View the diff for the pushed branch (${run.pushed_branch})`}
    >
      <ExternalLink className="w-3 h-3" /> View branch diff
    </a>
  );
};

const RemoteRunInbox: React.FC = () => {
  const [runs, setRuns] = useState<RemoteRunMirror[]>([]);
  const [machines, setMachines] = useState<Machine[]>([]);
  const [loading, setLoading] = useState(true);
  const [reconciling, setReconciling] = useState(false);
  const [error, setError] = useState<string>('');
  const { navigate } = useNavigation();

  const machineName = useCallback(
    (id: string) => machines.find((m) => m.id === id)?.name ?? id,
    [machines],
  );

  const load = useCallback(async () => {
    try {
      const [list, machineList] = await Promise.all([listMirroredRuns(), listMachines()]);
      setRuns(list ?? []);
      setMachines(machineList ?? []);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const reconcile = useCallback(async () => {
    setReconciling(true);
    setError('');
    try {
      const list = await reconcileRuns();
      setRuns(list ?? []);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setReconciling(false);
    }
  }, []);

  useEffect(() => {
    (async () => {
      await load();
      await reconcile();
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const grouped = useMemo(() => {
    const g: Record<Bucket, RemoteRunMirror[]> = {
      pr_ready: [], failed: [], parked: [], needs_credentials: [], running: [], unreachable: [], cancelled: [],
    };
    for (const r of runs) g[bucketFor(r.status)].push(r);
    for (const b of BUCKET_ORDER) g[b].sort((a, b2) => b2.updated_at - a.updated_at);
    return g;
  }, [runs]);

  const cancelRun = async (run: RemoteRunMirror) => {
    try {
      await cancelRemoteRun(run.machine_id, run.run_id);
      await reconcile();
    } catch (e) {
      setError(formatError(e));
    }
  };

  /** Every run is one Feature (eager shadow, M-C) — the run view is
   * FeatureDetail. `feature_id` can only be null on rows mirrored by a
   * pre-shadow app version before their first reconcile. */
  const openRun = (run: RemoteRunMirror) => {
    if (!run.feature_id) return;
    navigate({ kind: 'detail', featureId: run.feature_id, featureTitle: run.title });
  };

  const totalActionable = grouped.parked.length + grouped.needs_credentials.length + grouped.failed.length;

  return (
    <div className="flex-1 overflow-y-auto p-8 relative">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-cyan-600/5 rounded-full blur-[120px] pointer-events-none"></div>
      <div className="max-w-4xl mx-auto relative z-10">
        <div className="flex items-end justify-between mb-6 border-b border-white/5 pb-4">
          <div>
            <h2 className="text-2xl font-heading font-bold text-white mb-1 flex items-center gap-2">
              <Inbox className="w-6 h-6 text-cyan-400" />
              Runs
            </h2>
            <p className="text-sm text-slate-400">
              Every run launched on a remote machine, reconciled from each runner's own state.
              {totalActionable > 0 && (
                <span className="text-amber-300"> {totalActionable} need{totalActionable === 1 ? 's' : ''} your attention.</span>
              )}
            </p>
          </div>
          <button
            onClick={reconcile}
            disabled={reconciling}
            className="px-3 py-2 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5 disabled:opacity-50"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${reconciling ? 'animate-spin' : ''}`} />
            Refresh
          </button>
        </div>

        {error && (
          <div className="mb-4 text-[12px] text-red-300 bg-red-500/10 border border-red-500/20 rounded-lg p-3 break-all">
            {error}
          </div>
        )}

        {loading ? (
          <div className="text-center py-12 text-slate-500 text-sm">
            <Loader className="w-5 h-5 animate-spin mx-auto mb-2 text-cyan-400" />
            Loading…
          </div>
        ) : runs.length === 0 ? (
          <div className="glass-panel p-8 text-center flex flex-col items-center justify-center">
            <Inbox className="w-8 h-8 text-slate-500 mb-3" />
            <h3 className="text-base font-heading font-semibold text-white mb-1">No remote runs yet</h3>
            <p className="text-sm text-slate-400 max-w-md">
              Pick a remote machine under "Where to run" in the Start Feature modal and the run will show up here.
            </p>
          </div>
        ) : (
          <div className="space-y-6">
            {BUCKET_ORDER.filter((b) => grouped[b].length > 0).map((bucket) => {
              const meta = BUCKET_META[bucket];
              const accent = TONE_TEXT[meta.tone];
              const Icon = meta.icon;
              return (
                <div key={bucket}>
                  <div className="flex items-center gap-2 mb-2">
                    <Icon className={`w-4 h-4 ${accent}`} />
                    <h3 className={`text-xs font-mono uppercase tracking-wider ${accent}`}>{meta.label}</h3>
                    <span className="text-[10px] text-slate-600 font-mono">{grouped[bucket].length}</span>
                  </div>
                  <div className="space-y-2">
                    {grouped[bucket].map((run) => (
                      <div
                        key={`${run.machine_id}:${run.run_id}`}
                        onClick={() => openRun(run)}
                        className={`glass-panel p-3.5 flex items-start justify-between gap-4 border-l-2 ${TONE_BORDER_L[meta.tone]} ${
                          run.feature_id ? 'cursor-pointer hover:bg-white/[0.03] transition-colors' : ''
                        }`}
                        title={run.feature_id ? 'Open this run' : undefined}
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2 text-slate-200 text-sm font-medium">
                            <span className="truncate">{run.title}</span>
                            <Chip status={run.status} />
                          </div>
                          <div className="text-[11px] text-slate-500 font-mono mt-0.5 flex flex-wrap gap-x-3">
                            <span>{machineName(run.machine_id)}</span>
                            <span>{relativeTime(run.updated_at)}</span>
                            <span className="text-slate-600">run {run.run_id}</span>
                          </div>
                          {run.error && (
                            <p className="text-[11px] text-ruby-300/90 font-mono mt-1.5 break-words">{run.error}</p>
                          )}
                        </div>
                        {/* Quick actions act on the run without leaving the
                            hub — stopPropagation so they don't also trigger
                            the row's open-run navigation. */}
                        <div
                          className="shrink-0 flex items-center gap-2"
                          onClick={(e) => e.stopPropagation()}
                        >
                          <button
                            type="button"
                            onClick={() => openRun(run)}
                            disabled={!run.feature_id}
                            className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed"
                            title={
                              run.feature_id
                                ? 'Open the run — step timeline, activity log, artifacts, and cost'
                                : 'Mirrored by an older app version — refresh to link it to its feature'
                            }
                          >
                            <ListTree className="w-3 h-3" /> Open run
                          </button>
                          {bucket === 'pr_ready' && run.pr_url && (
                            <a
                              href={run.pr_url}
                              target="_blank"
                              rel="noreferrer"
                              className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-emerald-500/10 border border-emerald-500/30 hover:bg-emerald-500/20 text-emerald-300 flex items-center gap-1.5"
                            >
                              <ExternalLink className="w-3 h-3" /> Open PR
                            </a>
                          )}
                          {bucket !== 'pr_ready' && !run.pr_url && run.pushed_branch && (
                            <DiffLinkButton run={run} />
                          )}
                          {bucket === 'parked' && <RemoteGateActions run={run} onResolved={reconcile} />}
                          {bucket === 'needs_credentials' && (
                            <ReinjectCredentials run={run} onResolved={reconcile} />
                          )}
                          {(bucket === 'running' || bucket === 'parked') && (
                            <button
                              type="button"
                              onClick={() => cancelRun(run)}
                              className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5"
                              title="Cancel this run"
                            >
                              <Ban className="w-3 h-3" /> Cancel
                            </button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
};

export default RemoteRunInbox;
