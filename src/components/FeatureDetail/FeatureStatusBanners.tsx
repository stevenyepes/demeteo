import { AlertTriangle } from 'lucide-react';

interface FeatureStatusBannersProps {
  status: string;
}

/**
 * The awaiting_mr nudge.
 *
 * Sync used to be here too — a result banner, a review card, and the abort
 * button on one branch of the first. They are gone, not moved twice: a strip of
 * stacked notices above the run is where a state ends up when each phase adds
 * its own, and the pane in the inspector column is the one place that answers
 * "what is happening with this branch". The published PR's row went the same
 * way, into the header's status line: a full-width band for one link and its
 * state was a row of chrome that said less than the chip beside it.
 */
export function FeatureStatusBanners({ status }: FeatureStatusBannersProps) {
  if (status !== 'awaiting_mr') return null;
  return (
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
  );
}
