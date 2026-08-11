import React, { useEffect, useState } from 'react';
import { KeyRound, Loader, ThumbsDown, ThumbsUp } from 'lucide-react';
import type { RemoteRunMirror } from '../types';
import { formatError } from '../lib/errors';
import {
  decideRemoteGate,
  getRemoteRunStatus,
  parkedGateId,
  reinjectRemoteCredentials,
} from '../lib/remoteRuns';

/**
 * The two decisions a parked detached run can be unblocked with, factored out
 * of the activity surface they used to share a file with: both are rendered by
 * the Runs inbox as well as by `FeatureDetail`, and neither has anything to do
 * with the event log.
 */

/**
 * Approve/reject a detached run's parked gate from the laptop (M5.3's
 * `decide_gate` RPC). Resolves the live `gate_id` on mount via a fresh
 * `remote_get_status` — the mirror collapses it into a plain `"parked"`
 * status string.
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
 * resumes on its own.
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
