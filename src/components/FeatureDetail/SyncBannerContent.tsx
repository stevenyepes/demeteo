import React from 'react';
import { AlertTriangle, CheckCircle, Cpu, RefreshCw } from 'lucide-react';
import type { SyncOutcomeView } from '../../types';

/**
 * Render the most recent `feature_sync` / `feature_resolve_sync_conflicts`
 * result as an inline banner. The banner self-dismisses once the user
 * has acknowledged it (`onDismiss`).
 */
interface SyncBannerContentProps {
  outcome: SyncOutcomeView;
  onResolve: (files: string[]) => void;
  resolving: boolean;
  onDismiss: () => void;
}

export const SyncBannerContent: React.FC<SyncBannerContentProps> = ({
  outcome,
  onResolve,
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
            <span className="font-mono">{outcome.merge_commit_sha.slice(0, 7)}</span>
            {outcome.revalidated_step_id
              ? ' — re-validating the workflow.'
              : ' — run the validation step to confirm everything still builds.'}
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
        <div className="flex justify-end">
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
};
