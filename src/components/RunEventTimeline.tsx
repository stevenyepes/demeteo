import React, { useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronUp, KeyRound, Loader, Radio, ThumbsDown, ThumbsUp, WifiOff } from 'lucide-react';
import type { RemoteRunMirror, RunEvent } from '../types';
import { TERMINAL_STATUSES } from '../lib/runStatus';
import { formatError } from '../lib/errors';
import {
  decideRemoteGate,
  getRemoteRunStatus,
  parkedGateId,
  reinjectRemoteCredentials,
  streamRemoteEvents,
} from '../lib/remoteRuns';
import { RunEventFeed } from './RunEventFeed';

/**
 * Approve/reject a detached run's parked gate from the laptop (M5.3's
 * `decide_gate` RPC). Resolves the live `gate_id` on mount via a fresh
 * `remote_get_status` — the mirror collapses it into a plain `"parked"`
 * status string. Shared by the Runs inbox row and the FeatureDetail
 * Activity section, so gate decisions work wherever the run is shown.
 */
export const RemoteGateActions: React.FC<{ run: RemoteRunMirror; onResolved: () => void }> = ({
  run,
  onResolved,
}) => {
  const [gateId, setGateId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [deciding, setDeciding] = useState(false);
  const [err, setErr] = useState<string>('');

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await getRemoteRunStatus(run.machine_id, run.run_id);
        if (!cancelled) setGateId(parkedGateId(status));
      } catch (e) {
        if (!cancelled) setErr(formatError(e));
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
      await decideRemoteGate({
        machineId: run.machine_id,
        runId: run.run_id,
        gateId,
        decision,
        feedback: null,
      });
      onResolved();
    } catch (e) {
      setErr(formatError(e));
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

/**
 * Re-inject the git PAT for a run parked at `needs-credentials` (§7.1).
 * The runner keeps the credential in memory only, so a runner restart —
 * or an injection that failed right after submit — leaves the run waiting
 * for the laptop to re-supply it. `remote_reinject_credentials` resolves
 * the PAT from the run's project and pushes it over the tunnel; the runner
 * resumes on its own. Shared by the Runs inbox row and FeatureDetail so a
 * re-inject works wherever the run is shown.
 */
export const ReinjectCredentials: React.FC<{ run: RemoteRunMirror; onResolved: () => void }> = ({
  run,
  onResolved,
}) => {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string>('');

  const reinject = async () => {
    setBusy(true);
    setErr('');
    try {
      await reinjectRemoteCredentials(run.machine_id, run.run_id);
      onResolved();
    } catch (e) {
      setErr(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        onClick={reinject}
        disabled={busy}
        title="Re-send this machine's git credentials so the run can resume"
        className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-amber-500/10 border border-amber-500/30 hover:bg-amber-500/20 text-amber-200 flex items-center gap-1.5 disabled:opacity-50"
      >
        {busy ? <Loader className="w-3 h-3 animate-spin" /> : <KeyRound className="w-3 h-3" />}
        {busy ? 'Re-injecting…' : 'Re-inject credentials'}
      </button>
      {err && <span className="text-[10px] text-ruby-300 font-mono break-all">{err}</span>}
    </div>
  );
};

/**
 * Inline activity feed for a detached run: tails the runner's
 * append-only event log (`stream_events`, M3.3) while the run is live,
 * or fetches it once for a terminal run. This is "identical to a local
 * run" for what the control channel actually carries — the run's
 * coarse milestones — not a per-token agent transcript; the runner
 * doesn't stream raw agent stdout over the control channel today.
 *
 * Rendered as a content-sized collapsible panel inside FeatureDetail
 * (never a modal): the run view owns the run, the panel is one section
 * of it.
 */
export const RunEventTimeline: React.FC<{
  run: RemoteRunMirror;
  machineName: string;
  /** Notified with each batch of freshly-fetched events, so a parent can
   *  derive its own view (e.g. the bootstrap stepper) without a second poll
   *  of `remote_stream_events`. */
  onEvents?: (events: RunEvent[]) => void;
}> = ({
  run,
  machineName,
  onEvents,
}) => {
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [error, setError] = useState<string>('');
  const [open, setOpen] = useState(true);
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
  // instead of polling every 2s forever.
  const isTerminal = TERMINAL_STATUSES.includes(run.status);

  useEffect(() => {
    // Collapsed panel = nothing to paint; don't keep the 2s poll (and its
    // SSH round-trips) alive for it. `offsetRef` survives the collapse, so
    // reopening resumes from the last consumed event instead of refetching.
    if (!open) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const fresh = await streamRemoteEvents(run.machine_id, run.run_id, offsetRef.current);
        if (cancelled) return;
        setError('');
        setConsecutiveFailures(0);
        if (!fresh || fresh.length === 0) return;
        offsetRef.current = Math.max(offsetRef.current, ...fresh.map((e) => e.offset));
        onEvents?.(fresh);
        setEvents((prev) => [...prev, ...fresh]);
        setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }), 0);
      } catch (e) {
        if (cancelled) return;
        // A terminal run gets exactly one fetch attempt (no retry loop
        // to eventually succeed), so don't wait for a streak that will
        // never accumulate — surface the failure right away.
        if (isTerminal) {
          setError(formatError(e));
          return;
        }
        setConsecutiveFailures((n) => {
          const next = n + 1;
          if (next >= FAILURE_THRESHOLD) setError(formatError(e));
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
  }, [run.machine_id, run.run_id, isTerminal, open]);

  return (
    <div className="glass-panel border border-white/5 overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full px-4 py-3 flex items-center justify-between gap-2 text-left hover:bg-white/[0.02] transition-colors"
      >
        <div className="flex items-center gap-2 min-w-0">
          <Radio className={`w-4 h-4 shrink-0 ${error ? 'text-slate-600' : isTerminal ? 'text-slate-500' : 'text-cyan-400 animate-pulse'}`} />
          <span className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider">Activity</span>
          <span className="text-[10px] text-slate-500 font-mono truncate">{machineName} · run {run.run_id}</span>
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
          {open ? <ChevronUp className="w-4 h-4 text-slate-500" /> : <ChevronDown className="w-4 h-4 text-slate-500" />}
        </div>
      </button>
      {open && (
        <div className="max-h-64 overflow-y-auto px-4 pb-4 font-mono text-xs space-y-2 border-t border-white/5 pt-3">
          {error && (
            <p className="text-ruby-300 break-all">
              {isTerminal
                ? `Couldn't fetch the log from ${machineName}: ${error}.`
                : `Lost the connection to ${machineName}: ${error}. Still retrying every 2s — events shown so far are not lost.`}
            </p>
          )}
          {!error && <RunEventFeed events={events} />}
          <div ref={bottomRef} />
        </div>
      )}
    </div>
  );
};
