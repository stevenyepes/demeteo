/**
 * Loading placeholder for the project view's feature-pipeline list
 * (docs/UI_REDESIGN_PLAN.md §3.4). The centred spinner it replaces unmounted
 * the list, so every return to the view re-mounted every row; a placeholder
 * that keeps the box lets the section hold its position instead.
 *
 * `ROW_HEIGHT_PX` is derived from `PipelineCard`'s box rather than copied out
 * of its markup, which is being rewritten alongside this: 40px of `p-5`, a
 * ~20px chip row, `mb-1`, a 28px `text-lg` title line, and one description
 * line with its margin. A row with no description measures ~92px and one with
 * a two-line description ~135px, so no single number matches every row — this
 * one sits between them, which is the most a placeholder can do.
 *
 * Rows are flat shimmer blocks rather than `glass-panel` cards on purpose:
 * `backdrop-filter` is a budget on WebKitGTK, and spending a blur layer per
 * row on a surface nobody reads is the wrong place for it.
 */

import type { ReactElement } from 'react';

import { Skeleton } from './ui/Skeleton';

const ROW_HEIGHT_PX = 120;
const DEFAULT_ROWS = 3;

export interface PipelineListSkeletonProps {
  /** Rows to stand in for; clamped to at least one. */
  rows?: number;
  className?: string;
}

export function PipelineListSkeleton({
  rows = DEFAULT_ROWS,
  className = '',
}: PipelineListSkeletonProps): ReactElement {
  const count = Math.max(1, rows);

  return (
    <div
      data-testid="pipeline-list-skeleton"
      role="status"
      aria-busy="true"
      aria-label="Loading feature pipelines"
      className={`space-y-4 ${className}`}
    >
      {Array.from({ length: count }, (_, i) => `row-${i}`).map((key) => (
        <Skeleton key={key} variant="block" height={ROW_HEIGHT_PX} announce={false} />
      ))}
    </div>
  );
}
