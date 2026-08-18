import React from 'react';
import { AlertTriangle, CheckCircle, Cpu, RefreshCw, XCircle } from 'lucide-react';
import type { SyncBlockedStage, SyncOutcomeView } from '../../types';
import { TONE_TEXT } from '../../lib/runStatus';

/**
 * Render the most recent `feature_sync` / `feature_resolve_sync_conflicts`
 * result as an inline banner. The banner self-dismisses once the user
 * has acknowledged it (`onDismiss`).
 *
 * A blocked sync is amber, not ruby: nothing is left conflicted and no work is
 * lost, the sync just needs a human before it can run again — the tone
 * `lib/runStatus.ts` assigns to exactly that. Ruby would read as "your merge
 * failed" and send the user looking for damage that is not there.
 *
 * `raw_error` is rendered on both failure branches because it is the only
 * evidence the user has: the banner's own sentence is written here, git's is
 * the one that says which host refused and why.
 *
 * "Abort sync" is the other half of the conflict being durable: the merge and
 * its worktree now outlive this component, so there has to be somewhere to say
 * "not this one" — otherwise the only thing that ever cleans the tree up is
 * the next sync force-removing it.
 *
 * "Resolve with agent" does not test the file list. An empty list proves
 * nothing (`crate::domain::sync_failure`) — the porcelain read that fills it
 * answers empty on any transport error — and hiding the button on one leaves
 * a real conflict with no entry point anywhere in `src/`, while the workflow's
 * own sync step spawns the resolver on that identical value. The turn refuses
 * honestly ("No active merge in progress") when there is nothing to resolve.
 */
interface SyncBannerContentProps {
  outcome: SyncOutcomeView;
  onResolve: (files: string[]) => void;
  onAbort: () => void;
  resolving: boolean;
  onDismiss: () => void;
}

const BLOCKED_NEXT_MOVE: Record<SyncBlockedStage, string> = {
  fetch: "Could not reach origin. Check the project's remote and credentials.",
  base_ref_missing: "The base branch does not exist on origin. This run's base may be wrong.",
  worktree_provision: 'Could not create the sync worktree.',
  merge: 'The merge was cut short by the connection or a timeout, and never finished.',
  push: 'Merged locally, but the push to origin failed. The merge is not published.',
  repo_context: 'This feature has no project repository configured.',
};

export const SyncBannerContent: React.FC<SyncBannerContentProps> = ({
  outcome,
  onResolve,
  onAbort,
  resolving,
  onDismiss,
}) => {
  if (outcome.status === 'ok') {
    return (
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-emerald-400">
          <CheckCircle className="w-3.5 h-3.5" />
          <span>
            Synced with main.{' '}
            {outcome.changed
              ? `Merge commit ${outcome.merge_commit_sha.slice(0, 7)} created.`
              : 'No new commits upstream.'}
          </span>
        </div>
        <button
          onClick={onDismiss}
          className="text-slate-500 hover:text-white text-[10px] uppercase font-bold"
        >
          Dismiss
        </button>
      </div>
    );
  }
  if (outcome.status === 'resolved') {
    return (
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-emerald-400">
          <CheckCircle className="w-3.5 h-3.5" />
          <span>
            Conflicts resolved. Merge commit{' '}
            <span className="font-mono">{outcome.merge_commit_sha.slice(0, 7)}</span> — run the
            validation step to confirm everything still builds.
          </span>
        </div>
        <button
          onClick={onDismiss}
          className="text-slate-500 hover:text-white text-[10px] uppercase font-bold"
        >
          Dismiss
        </button>
      </div>
    );
  }
  if (outcome.status === 'blocked') {
    return (
      <div className="space-y-2">
        <div className="flex items-center justify-between gap-3">
          <div className={`flex items-center gap-2 ${TONE_TEXT.amber}`}>
            <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
            <span>
              <strong>Sync did not complete.</strong> {BLOCKED_NEXT_MOVE[outcome.stage]}
            </span>
          </div>
          <button
            onClick={onDismiss}
            className="text-slate-500 hover:text-white text-[10px] uppercase font-bold"
          >
            Dismiss
          </button>
        </div>
        <pre className="font-mono text-[11px] text-slate-300 whitespace-pre-wrap max-h-32 overflow-y-auto bg-black/30 p-2 rounded">
          {outcome.raw_error}
        </pre>
      </div>
    );
  }
  if (outcome.status === 'conflict') {
    return (
      <div className="space-y-2">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-rose-400">
            <AlertTriangle className="w-3.5 h-3.5" />
            <span>
              <strong>Merge conflict in {outcome.conflict_files.length} file(s).</strong>{' '}
              Resolve manually or spawn a fresh agent to clean up the markers.
            </span>
          </div>
          <button
            onClick={onDismiss}
            className="text-slate-500 hover:text-white text-[10px] uppercase font-bold"
          >
            Dismiss
          </button>
        </div>
        <ul className="font-mono text-[11px] text-slate-300 list-disc pl-5 max-h-32 overflow-y-auto bg-black/30 p-2 rounded">
          {outcome.conflict_files.map((f) => (
            <li key={f.path}>
              <span className="text-rose-300">{f.path}</span>
              <span className="text-slate-500"> — {f.kind}</span>
            </li>
          ))}
        </ul>
        <pre className="font-mono text-[11px] text-slate-300 whitespace-pre-wrap max-h-32 overflow-y-auto bg-black/30 p-2 rounded">
          {outcome.raw_error}
        </pre>
        <div className="flex justify-end gap-2">
          <button
            onClick={onAbort}
            disabled={resolving}
            className="flex items-center gap-1.5 px-3 py-1.5 border border-rose-500/30 hover:bg-rose-500/10 rounded text-xs font-bold text-rose-300 transition disabled:opacity-40"
            title="Undo the merge and discard the sync worktree"
          >
            <XCircle className="w-3 h-3" />
            Abort sync
          </button>
          <button
            onClick={() => onResolve(outcome.conflict_files.map((f) => f.path))}
            disabled={resolving}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-violet-600 hover:bg-violet-500 hover:shadow-[0_0_20px_rgba(139,92,246,0.5)] rounded text-xs font-bold text-white transition disabled:opacity-40"
          >
            {resolving ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Cpu className="w-3 h-3" />}
            Resolve with agent
          </button>
        </div>
      </div>
    );
  }
  if (outcome.status === 'resolution_failed') {
    return (
      <div className="space-y-2">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-rose-400">
            <AlertTriangle className="w-3.5 h-3.5" />
            <span>
              <strong>Resolver failed.</strong> {outcome.reason}
            </span>
          </div>
          <button
            onClick={onDismiss}
            className="text-slate-500 hover:text-white text-[10px] uppercase font-bold"
          >
            Dismiss
          </button>
        </div>
        <ul className="font-mono text-[11px] text-slate-300 list-disc pl-5 max-h-32 overflow-y-auto bg-black/30 p-2 rounded">
          {outcome.conflict_files.map((f) => (
            <li key={f.path}>
              <span className="text-rose-300">{f.path}</span>
              <span className="text-slate-500"> — {f.kind}</span>
            </li>
          ))}
        </ul>
      </div>
    );
  }
  return null;
};
