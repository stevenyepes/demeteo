import { AlertTriangle, ExternalLink, GitPullRequest } from 'lucide-react';
import type { MrState } from '../../types';

interface FeatureStatusBannersProps {
  status: string;
  mrUrl: string | null;
  mrState: MrState | null;
  onRefreshMrState: () => void;
}

/**
 * The awaiting_mr nudge and the published PR/MR row.
 *
 * Sync used to be here too — a result banner, a review card, and the abort
 * button on one branch of the first. They are gone, not moved twice: a strip of
 * stacked notices above the run is where a state ends up when each phase adds
 * its own, and the pane in the inspector column is the one place that answers
 * "what is happening with this branch". What is left is the two notices that
 * describe the *pull request*, which is the run's output rather than its
 * branch's state.
 */
export function FeatureStatusBanners({
  status,
  mrUrl,
  mrState,
  onRefreshMrState,
}: FeatureStatusBannersProps) {
  return (
    <>
      {status === 'awaiting_mr' && (
        <div className="px-6 py-3 bg-amber-500/5 border-b border-amber-500/20 flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-amber-400 text-xs">
            <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
            <span>
              <strong className="font-bold">All steps complete, but no PR was opened.</strong>{' '}
              This feature's workflow has no finalize step, or the publish didn't
              go through. Publish above to open one — the agent's summary is used
              if it wrote one.
            </span>
          </div>
        </div>
      )}

      {mrUrl && (
        <div className="px-6 py-2 bg-[#0d0f14]/40 border-b border-white/5 flex items-center justify-between gap-3 text-xs">
          <div className="flex items-center gap-2 text-slate-300">
            <GitPullRequest className="w-3.5 h-3.5 text-cyan-400" />
            <span className="font-mono text-cyan-400">{mrState ?? 'unknown'}</span>
            <a
              href={mrUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-slate-400 hover:text-white flex items-center gap-1 transition"
            >
              {mrUrl.length > 60 ? `${mrUrl.slice(0, 57)}…` : mrUrl}
              <ExternalLink className="w-3 h-3" />
            </a>
          </div>
          <button
            onClick={onRefreshMrState}
            className="text-[10px] uppercase tracking-wider text-slate-500 hover:text-white transition font-bold"
            title="Refresh MR state from the provider"
          >
            Refresh
          </button>
        </div>
      )}
    </>
  );
}
