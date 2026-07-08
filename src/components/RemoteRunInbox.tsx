import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
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
  ThumbsUp,
  ThumbsDown,
  Inbox,
  Radio,
  X,
  ListTree,
} from 'lucide-react';
import type { Machine, RemoteRunMirror, RunEvent } from '../types';
import { useNavigation } from '../context';

/**
 * Return inbox (docs/REMOTE_EXECUTION_PLAN.md M6.2, design §8). Groups
 * every mirrored remote run into the taxonomy from the design doc's
 * table: PR ready / Failed / Parked / Needs credentials / Running /
 * Unreachable. `cancelled` isn't in that table (it's a deliberate user
 * action, not an outcome to chase) so it gets its own low-priority
 * bucket rather than being crowbarred into "Failed".
 */

export type Bucket =
  | 'pr_ready'
  | 'failed'
  | 'parked'
  | 'needs_credentials'
  | 'running'
  | 'unreachable'
  | 'cancelled';

const BUCKET_ORDER: Bucket[] = [
  'parked',
  'needs_credentials',
  'failed',
  'pr_ready',
  'running',
  'unreachable',
  'cancelled',
];

const BUCKET_META: Record<
  Bucket,
  { label: string; icon: React.ComponentType<{ className?: string }>; accent: string; border: string }
> = {
  pr_ready: { label: 'PR ready', icon: CheckCircle2, accent: 'text-emerald-400', border: 'border-l-emerald-500/60' },
  failed: { label: 'Failed', icon: XCircle, accent: 'text-ruby-400', border: 'border-l-ruby-500/60' },
  parked: { label: 'Parked — needs you', icon: PauseCircle, accent: 'text-amber-400', border: 'border-l-amber-500/60' },
  needs_credentials: { label: 'Needs credentials', icon: KeyRound, accent: 'text-amber-400', border: 'border-l-amber-500/60' },
  running: { label: 'Running', icon: Loader, accent: 'text-cyan-400', border: 'border-l-cyan-500/60' },
  unreachable: { label: 'Unreachable', icon: WifiOff, accent: 'text-slate-500', border: 'border-l-slate-600/60' },
  cancelled: { label: 'Cancelled', icon: Ban, accent: 'text-slate-500', border: 'border-l-slate-600/60' },
};

export const TERMINAL_STATUSES = ['failed', 'cancelled', 'awaiting_mr', 'completed'];

export function bucketFor(status: string): Bucket {
  switch (status) {
    case 'awaiting_mr':
    case 'completed':
      return 'pr_ready';
    case 'failed':
    case 'interrupted':
      return 'failed';
    case 'parked':
    case 'over-budget':
      return 'parked';
    case 'needs-credentials':
      return 'needs_credentials';
    case 'unreachable':
      return 'unreachable';
    case 'cancelled':
      return 'cancelled';
    case 'pending':
    case 'running':
    default:
      return 'running';
  }
}

function relativeTime(ms: number): string {
  const deltaSec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (deltaSec < 60) return 'just now';
  const deltaMin = Math.floor(deltaSec / 60);
  if (deltaMin < 60) return `${deltaMin}m ago`;
  const deltaHr = Math.floor(deltaMin / 60);
  if (deltaHr < 24) return `${deltaHr}h ago`;
  return `${Math.floor(deltaHr / 24)}d ago`;
}

/**
 * "View branch diff" (docs/REMOTE_EXECUTION_PLAN.md M6.2 follow-up): a
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
    invoke<string | null>('remote_run_diff_url', { projectId: run.project_id, branch: run.pushed_branch })
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

const ParkedRow: React.FC<{ run: RemoteRunMirror; onResolved: () => void }> = ({ run, onResolved }) => {
  const [gateId, setGateId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [deciding, setDeciding] = useState(false);
  const [err, setErr] = useState<string>('');

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status: any = await invoke('remote_get_status', { machineId: run.machine_id, runId: run.run_id });
        if (!cancelled) setGateId(status?.parked_gate_id ?? null);
      } catch (e) {
        if (!cancelled) setErr(String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [run.machine_id, run.run_id]);

  const decide = async (decision: 'approve' | 'reject') => {
    if (!gateId) return;
    setDeciding(true);
    try {
      await invoke('remote_decide_gate', {
        machineId: run.machine_id,
        runId: run.run_id,
        gateId,
        decision,
        feedback: null,
      });
      onResolved();
    } catch (e) {
      setErr(String(e));
    } finally {
      setDeciding(false);
    }
  };

  if (loading) {
    return <span className="text-[11px] text-slate-500 font-mono">Checking gate…</span>;
  }
  if (!gateId) {
    // `over-budget` parks too but has no gate to decide — just point at the inbox refresh.
    return <span className="text-[11px] text-slate-500 font-mono">Over budget — clear the cap to resume.</span>;
  }
  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        onClick={() => decide('approve')}
        disabled={deciding}
        className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-emerald-500/10 border border-emerald-500/30 hover:bg-emerald-500/20 text-emerald-300 flex items-center gap-1.5 disabled:opacity-50"
      >
        <ThumbsUp className="w-3 h-3" /> Approve
      </button>
      <button
        type="button"
        onClick={() => decide('reject')}
        disabled={deciding}
        className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-ruby-500/10 border border-ruby-500/30 hover:bg-ruby-500/20 text-ruby-300 flex items-center gap-1.5 disabled:opacity-50"
      >
        <ThumbsDown className="w-3 h-3" /> Reject
      </button>
      {err && <span className="text-[10px] text-ruby-300 font-mono break-all">{err}</span>}
    </div>
  );
};

/** Best-effort human-readable rendering of a `RunEvent.payload_json` —
 *  most kinds carry a plain JSON string (title, branch name, url, …);
 *  a few (StepProgress-like) could carry structured JSON in the
 *  future, so this falls back to the raw text for anything that
 *  doesn't parse as a bare string. */
function formatPayload(payloadJson: string | null): string {
  if (!payloadJson) return '';
  try {
    const parsed = JSON.parse(payloadJson);
    return typeof parsed === 'string' ? parsed : JSON.stringify(parsed);
  } catch {
    return payloadJson;
  }
}

const EVENT_KIND_LABEL: Record<string, string> = {
  submitted: 'Submitted',
  project_created: 'Project bootstrapped',
  bootstrapped: 'Repository cloned',
  feature_started: 'Feature started',
  gate_auto_approved: 'Gate auto-approved',
  parked: 'Parked — needs a decision',
  over_budget: 'Over budget',
  needs_credentials: 'Needs credentials',
  cost: 'Total cost',
  terminal_state: 'Reached terminal state',
  pushed: 'Branch pushed',
  pr_opened: 'PR opened',
  pr_open_failed: 'PR failed to open',
  cancelled: 'Cancelled',
};

/**
 * Live view (M6.4): tails a remote run's append-only event log
 * (`stream_events`, M3.3) while open. This is "identical to a local
 * run" for what the control channel actually carries — the run's
 * coarse milestones — not a per-token agent transcript; the runner
 * doesn't stream raw agent stdout over the control channel today.
 */
const LiveEventView: React.FC<{ run: RemoteRunMirror; machineName: string; onClose: () => void }> = ({
  run,
  machineName,
  onClose,
}) => {
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [error, setError] = useState<string>('');
  // A single blip (tunnel hiccup, one dropped poll) shouldn't paint a
  // permanent red banner over an otherwise-live view — only surface an
  // error once a few consecutive polls have failed, and always clear it
  // the moment a poll succeeds again. Below that threshold, show a
  // quieter "Reconnecting…" state instead of nothing/stale silence.
  const [consecutiveFailures, setConsecutiveFailures] = useState(0);
  const FAILURE_THRESHOLD = 3;
  const offsetRef = useRef(0);
  const bottomRef = useRef<HTMLDivElement>(null);

  // A terminal run's log can't change any further — fetch it once
  // instead of polling every 2s forever, which is what previously made
  // "View log" on a failed/finished run indistinguishable from tailing
  // a live one.
  const isTerminal = TERMINAL_STATUSES.includes(run.status);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const fresh = await invoke<RunEvent[]>('remote_stream_events', {
          machineId: run.machine_id,
          runId: run.run_id,
          fromOffset: offsetRef.current,
        });
        if (cancelled) return;
        setError('');
        setConsecutiveFailures(0);
        if (!fresh || fresh.length === 0) return;
        offsetRef.current = Math.max(offsetRef.current, ...fresh.map((e) => e.offset));
        setEvents((prev) => [...prev, ...fresh]);
        setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: 'smooth' }), 0);
      } catch (e) {
        if (cancelled) return;
        // A terminal run gets exactly one fetch attempt (no retry loop
        // to eventually succeed), so don't wait for a streak that will
        // never accumulate — surface the failure right away.
        if (isTerminal) {
          setError(String(e));
          return;
        }
        setConsecutiveFailures((n) => {
          const next = n + 1;
          if (next >= FAILURE_THRESHOLD) setError(String(e));
          return next;
        });
      }
    };
    poll();
    if (isTerminal) return () => { cancelled = true; };
    const interval = setInterval(poll, 2000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [run.machine_id, run.run_id, isTerminal]);

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-[70] p-4">
      <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-lg h-[70vh] shadow-2xl overflow-hidden flex flex-col">
        <div className="px-5 py-4 border-b border-white/5 flex items-center justify-between bg-[#050508]">
          <div className="flex items-center gap-2 min-w-0">
            <Radio className={`w-4 h-4 shrink-0 ${error ? 'text-slate-600' : isTerminal ? 'text-slate-500' : 'text-cyan-400 animate-pulse'}`} />
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-white truncate">{run.title}</h3>
              <p className="text-[10px] text-slate-500 font-mono">{machineName} · run {run.run_id}</p>
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {consecutiveFailures > 0 && !error && (
              <span className="text-[10px] text-amber-300 flex items-center gap-1">
                <Loader className="w-3 h-3 animate-spin" /> Reconnecting…
              </span>
            )}
            {error && (
              <span className="text-[10px] text-slate-500 flex items-center gap-1">
                <WifiOff className="w-3 h-3" /> Disconnected
              </span>
            )}
            <button type="button" onClick={onClose} className="text-slate-500 hover:text-white transition-colors" aria-label="Close">
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>
        <div className="flex-1 overflow-y-auto p-4 font-mono text-xs space-y-2">
          {error && (
            <p className="text-ruby-300 break-all">
              Lost the connection to {machineName}: {error}. Still retrying every 2s — events shown so far are not lost.
            </p>
          )}
          {events.length === 0 && !error && (
            <p className="text-slate-500">Waiting for events…</p>
          )}
          {events.map((e) => (
            <div key={e.offset} className="flex items-start gap-2">
              <span className="text-slate-600 shrink-0">{new Date(e.created_at).toLocaleTimeString()}</span>
              <div className="min-w-0">
                <span className="text-cyan-300">{EVENT_KIND_LABEL[e.kind] ?? e.kind}</span>
                {e.payload_json && (
                  <span className="text-slate-400 ml-1.5 break-words">{formatPayload(e.payload_json)}</span>
                )}
              </div>
            </div>
          ))}
          <div ref={bottomRef} />
        </div>
      </div>
    </div>
  );
};

const RemoteRunInbox: React.FC = () => {
  const [runs, setRuns] = useState<RemoteRunMirror[]>([]);
  const [machines, setMachines] = useState<Machine[]>([]);
  const [loading, setLoading] = useState(true);
  const [reconciling, setReconciling] = useState(false);
  const [error, setError] = useState<string>('');
  const [liveRun, setLiveRun] = useState<RemoteRunMirror | null>(null);
  const { navigate } = useNavigation();

  const machineName = useCallback(
    (id: string) => machines.find((m) => m.id === id)?.name ?? id,
    [machines],
  );

  const load = useCallback(async () => {
    try {
      const [list, machineList] = await Promise.all([
        invoke<RemoteRunMirror[]>('remote_list_mirrored_runs'),
        invoke<Machine[]>('get_machines'),
      ]);
      setRuns(list ?? []);
      setMachines(machineList ?? []);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const reconcile = useCallback(async () => {
    setReconciling(true);
    setError('');
    try {
      const list = await invoke<RemoteRunMirror[]>('remote_reconcile_runs');
      setRuns(list ?? []);
    } catch (e) {
      setError(String(e));
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
      await invoke('remote_cancel_run', { machineId: run.machine_id, runId: run.run_id });
      await reconcile();
    } catch (e) {
      setError(String(e));
    }
  };

  const totalActionable = grouped.parked.length + grouped.needs_credentials.length + grouped.failed.length;

  return (
    <div className="flex-1 overflow-y-auto p-8 relative">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-cyan-600/5 rounded-full blur-[120px] pointer-events-none"></div>
      <div className="max-w-4xl mx-auto relative z-10">
        <div className="flex items-end justify-between mb-6 border-b border-white/5 pb-4">
          <div>
            <h2 className="text-2xl font-outfit font-bold text-white mb-1 flex items-center gap-2">
              <Inbox className="w-6 h-6 text-cyan-400" />
              Return inbox
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
            <h3 className="text-base font-outfit font-semibold text-white mb-1">No remote runs yet</h3>
            <p className="text-sm text-slate-400 max-w-md">
              Launch a feature with "Run on machine" set in the Start Feature modal and it'll show up here.
            </p>
          </div>
        ) : (
          <div className="space-y-6">
            {BUCKET_ORDER.filter((b) => grouped[b].length > 0).map((bucket) => {
              const meta = BUCKET_META[bucket];
              const Icon = meta.icon;
              return (
                <div key={bucket}>
                  <div className="flex items-center gap-2 mb-2">
                    <Icon className={`w-4 h-4 ${meta.accent}`} />
                    <h3 className={`text-xs font-mono uppercase tracking-wider ${meta.accent}`}>{meta.label}</h3>
                    <span className="text-[10px] text-slate-600 font-mono">{grouped[bucket].length}</span>
                  </div>
                  <div className="space-y-2">
                    {grouped[bucket].map((run) => (
                      <div
                        key={`${run.machine_id}:${run.run_id}`}
                        className={`glass-panel p-3.5 flex items-start justify-between gap-4 border-l-2 ${meta.border}`}
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2 text-slate-200 text-sm font-medium">
                            <span className="truncate">{run.title}</span>
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
                        <div className="shrink-0 flex items-center gap-2">
                          {run.feature_id && (
                            <button
                              type="button"
                              onClick={() =>
                                navigate({ kind: 'detail', featureId: run.feature_id!, featureTitle: run.title })
                              }
                              className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5"
                              title="Open the full feature view — steps, artifacts, and cost mirrored from the runner (C4.3)"
                            >
                              <ListTree className="w-3 h-3" /> View feature
                            </button>
                          )}
                          {(bucket === 'running' || bucket === 'parked') && (
                            <button
                              type="button"
                              onClick={() => setLiveRun(run)}
                              className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-cyan-500/10 border border-cyan-500/30 hover:bg-cyan-500/20 text-cyan-300 flex items-center gap-1.5"
                              title="Tail this run's live event log"
                            >
                              <Radio className="w-3 h-3" /> Live
                            </button>
                          )}
                          {bucket !== 'running' && bucket !== 'parked' && (
                            <button
                              type="button"
                              onClick={() => setLiveRun(run)}
                              className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5"
                              title="View this run's immutable audit trail — every auto-approved gate, budget check, and cost, as the runner recorded it"
                            >
                              <ListTree className="w-3 h-3" /> Audit log
                            </button>
                          )}
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
                          {bucket === 'parked' && <ParkedRow run={run} onResolved={reconcile} />}
                          {bucket === 'needs_credentials' && (
                            <span className="text-[11px] text-slate-500 font-mono">
                              Reconnect to this machine to re-inject credentials.
                            </span>
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
      {liveRun && (
        <LiveEventView run={liveRun} machineName={machineName(liveRun.machine_id)} onClose={() => setLiveRun(null)} />
      )}
    </div>
  );
};

export default RemoteRunInbox;
