import { AlertTriangle } from 'lucide-react';

import { stripAnsi } from '../../lib/rawOutput';

interface FeatureStatusBannersProps {
  status: string;
  /** Why the runner's push or PR open failed, for a detached run whose feature
   *  finished. The feature row still reads `awaiting_mr` and every step reads
   *  completed, so without this the run's only failure is invisible here. */
  publishError?: string | null;
}

/** A failing pre-push hook can print a whole test suite; its end says why. */
const ERROR_TAIL_LINES = 20;

function errorTail(raw: string): string {
  const lines = stripAnsi(raw).trimEnd().split('\n');
  return lines.slice(-ERROR_TAIL_LINES).join('\n');
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
export function FeatureStatusBanners({ status, publishError }: FeatureStatusBannersProps) {
  if (status !== 'awaiting_mr') return null;
  if (publishError) {
    return (
      <div className="px-6 py-3 bg-ruby-500/5 border-b border-ruby-500/20 space-y-2">
        <div className="flex items-center gap-2 text-ruby-300 text-xs">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
          <span>
            <strong className="font-bold">All steps complete, but the publish failed, so no PR was opened.</strong>{' '}
            Fix what the output below names, then Publish above to push and open the PR again.
          </span>
        </div>
        <pre className="max-h-48 overflow-auto rounded-md border border-ruby-500/20 bg-ruby-500/5 px-3 py-2 font-mono text-[11px] text-ruby-200 whitespace-pre-wrap break-words">
          {errorTail(publishError)}
        </pre>
      </div>
    );
  }
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
