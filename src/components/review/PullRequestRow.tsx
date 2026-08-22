import { memo, useCallback } from 'react';
import { ChevronRight } from 'lucide-react';

import { Chip } from '../ui/Chip';
import { PullRequestLaunch } from './PullRequestLaunch';
import { describePullRequestRow } from '../../lib/pullRequestRow';
import type { PullRequestSummary } from '../../lib/pullRequests';
import type { ReviewLaunchParams } from '../../lib/reviewLaunch';

export interface PullRequestRowProps {
  pullRequest: PullRequestSummary;
  onReview: (params: ReviewLaunchParams) => Promise<void>;
  /** The project's stored default harness — the one a review launched here
   *  actually runs on. Empty until the settings read lands, or when the project
   *  has stored none. */
  agentKind: string;
  /**
   * Ask for this row's review tier — mergeability and the diffstat, which no
   * list endpoint carries for GitHub. Called when the user points at the row
   * rather than on mount, because the alternative is one provider request per
   * row per refresh; `CodeReviewView` records the rate-limit reasoning and
   * makes a repeat call free.
   */
  onRequestDetail?: (pullRequestUrl: string) => void;
  /** Pinned by tests; a real row reads the clock at render. Left off the
   *  parent's call on purpose — a `Date.now()` passed down changes every
   *  render and would defeat the memo below. */
  now?: number;
}

/**
 * One open pull request, in three tiers of weight (`lib/pullRequestRow.ts`
 * decides what each says). A list of equally-loud lines is a wall of text that
 * has to be read in full to find one row, which is the opposite of what a
 * review queue is for.
 *
 * The branch pair is a plain span rather than a `Chip`: chips are uppercase by
 * construction, and a branch name is a case-sensitive identifier — `Fix-Auth`
 * rendered as `FIX-AUTH` names a branch that does not exist.
 *
 * The launch footer sits outside the link rather than inside it: a button and a
 * textarea nested in an anchor are interactive content inside interactive
 * content, which browsers resolve by handing the click to whichever they feel
 * like — here, by opening the provider in a browser instead of typing.
 */
function PullRequestRowImpl({
  pullRequest,
  onReview,
  agentKind,
  onRequestDetail,
  now,
}: PullRequestRowProps): React.ReactElement {
  const row = describePullRequestRow(pullRequest, now);
  const url = pullRequest.web_url;
  const requestDetail = useCallback(() => onRequestDetail?.(url), [onRequestDetail, url]);

  return (
    <div
      className="group rounded-xl border border-white/5 bg-black/20 transition-colors hover:border-cyan-500/30"
      onMouseEnter={requestDetail}
      onFocusCapture={requestDetail}
    >
      <a
        href={row.url}
        target="_blank"
        rel="noreferrer"
        data-testid="pull-request-row"
        className="block rounded-t-xl p-4 transition-colors hover:bg-white/5"
      >
        <div className="flex items-start gap-3">
          <div className="min-w-0 flex-1 space-y-2">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="font-heading text-sm font-medium text-white line-clamp-2">
                {row.title}
              </h3>
              <span className="font-mono text-xs text-slate-500">{row.number}</span>
              {row.chips.map((chip) => (
                <Chip key={chip.label} tone={chip.tone} size="sm" dot={false}>
                  {chip.label}
                </Chip>
              ))}
              {row.updatedAgo && (
                <span className="text-[11px] text-slate-500">{row.updatedAgo}</span>
              )}
            </div>

            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-slate-400">
              <span className="rounded border border-white/10 bg-white/5 px-1.5 py-0.5 text-slate-300">
                {row.branchLabel}
              </span>
              {row.author && <span>{row.author}</span>}
              {row.diffstat && (
                <span className="flex items-center gap-1.5">
                  <span className="text-emerald-400">{row.diffstat.additions}</span>
                  <span className="text-ruby-400">{row.diffstat.deletions}</span>
                </span>
              )}
              {row.fileLabel && <span>{row.fileLabel}</span>}
            </div>

            {row.timeline && <p className="text-[11px] text-slate-500">{row.timeline}</p>}
          </div>

          <ChevronRight
            aria-hidden="true"
            className="mt-1 w-4 h-4 shrink-0 text-slate-500 opacity-0 transition-opacity group-hover:opacity-100"
          />
        </div>
      </a>

      <PullRequestLaunch pullRequest={pullRequest} onReview={onReview} agentKind={agentKind} />
    </div>
  );
}

export const PullRequestRow = memo(PullRequestRowImpl);

export default PullRequestRow;
