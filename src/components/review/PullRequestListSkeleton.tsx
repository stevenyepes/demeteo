import type { ReactElement } from 'react';

import { Skeleton } from '../ui/Skeleton';

const ROW_HEIGHT_PX = 96;
const DEFAULT_ROWS = 4;

export interface PullRequestListSkeletonProps {
  rows?: number;
}

/**
 * Placeholder rows for the pull-request list. Flat blocks, not `glass-panel`
 * cards, for the reason `PipelineListSkeleton` records: a `backdrop-filter`
 * layer per row is real GPU cost on WebKitGTK, and a surface nobody reads is
 * the wrong place to spend it.
 *
 * `ROW_HEIGHT_PX` stands in for `PullRequestRow`'s three tiers — a two-line
 * title, the mono context line, the timeline — rather than a round number.
 */
export function PullRequestListSkeleton({
  rows = DEFAULT_ROWS,
}: PullRequestListSkeletonProps): ReactElement {
  return (
    <div
      data-testid="pull-request-list-skeleton"
      role="status"
      aria-busy="true"
      aria-label="Loading open pull requests"
      className="space-y-3"
    >
      {Array.from({ length: Math.max(1, rows) }, (_, i) => `row-${i}`).map((key) => (
        <Skeleton key={key} variant="block" height={ROW_HEIGHT_PX} announce={false} />
      ))}
    </div>
  );
}

export default PullRequestListSkeleton;
