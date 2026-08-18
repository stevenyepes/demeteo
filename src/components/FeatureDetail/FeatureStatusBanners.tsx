import { AlertTriangle, ExternalLink, GitPullRequest } from 'lucide-react';
import type { MrState, SyncOutcomeView } from '../../types';
import type { SyncResolverChoice } from '../../lib/featureSync';
import { SyncBannerContent } from './SyncBannerContent';
import type { SyncResolverSelection } from './useSyncResolverOverrides';

interface FeatureStatusBannersProps {
  status: string;
  syncBanner: SyncOutcomeView | null;
  resolving: boolean;
  aborting: boolean;
  onResolveConflicts: (files: string[], resolver: SyncResolverChoice) => void;
  /** The banner's own harness selection — see `SyncBannerContent`. */
  resolverSelection: SyncResolverSelection;
  onAbortSync: () => void;
  onDismissSyncBanner: () => void;
  mrUrl: string | null;
  mrState: MrState | null;
  onRefreshMrState: () => void;
}

/** The awaiting_mr nudge, the sync result, and the published PR/MR row. */
export function FeatureStatusBanners({
  status,
  syncBanner,
  resolving,
  aborting,
  onResolveConflicts,
  resolverSelection,
  onAbortSync,
  onDismissSyncBanner,
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

      {syncBanner && (
        <div className={`px-6 py-3 border-b flex items-start gap-3 ${
          syncBanner.status === 'ok' ? 'bg-emerald-500/5 border-emerald-500/20' :
          syncBanner.status === 'resolved' ? 'bg-emerald-500/5 border-emerald-500/20' :
          syncBanner.status === 'blocked' ? 'bg-amber-500/5 border-amber-500/20' :
          'bg-rose-500/5 border-rose-500/20'
        }`}>
          <div className="flex-1 text-xs text-slate-200 space-y-2">
            <SyncBannerContent
              outcome={syncBanner}
              onResolve={onResolveConflicts}
              resolverSelection={resolverSelection}
              onAbort={onAbortSync}
              resolving={resolving}
              aborting={aborting}
              onDismiss={onDismissSyncBanner}
            />
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
