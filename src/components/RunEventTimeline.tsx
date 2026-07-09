import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ChevronDown, ChevronUp, Loader, Radio, ThumbsDown, ThumbsUp, WifiOff } from 'lucide-react';
import type { RemoteRunMirror, RunEvent } from '../types';
import { TERMINAL_STATUSES } from '../lib/runStatus';

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
export const RunEventTimeline: React.FC<{ run: RemoteRunMirror; machineName: string }> = ({
  run,
  machineName,
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
        setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }), 0);
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
      )}
    </div>
  );
};
