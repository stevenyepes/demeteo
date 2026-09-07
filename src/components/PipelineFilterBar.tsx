import React, { useCallback, useMemo } from 'react';

import {
  clearFilters,
  segmentCounts,
  type PipelineFilterOptions,
  type PipelineRow,
  type PipelineSegment,
  type PipelineSort,
} from '../lib/pipelineFilter';
import { ListFilterBar } from './ui/ListFilterBar';
import type { SegmentedOption } from './ui/SegmentedControl';

export interface PipelineFilterBarProps {
  value: PipelineFilterOptions;
  onChange: (next: PipelineFilterOptions) => void;
  /** The whole unfiltered list — `segmentCounts` is deliberately taken over it. */
  features: readonly PipelineRow[];
  /** What `filterPipelines` kept, so the bar can own the empty state its own
   *  controls caused without recomputing the filter. */
  resultCount: number;
  className?: string;
}

/**
 * Segment + query + sort for the feature-pipeline list (UI_REDESIGN_PLAN §3.2).
 *
 * Every rule about *which* rows survive lives in `lib/pipelineFilter.ts`, and
 * every rule about how the controls look and behave in
 * `ui/ListFilterBar.tsx` — shared with the Discovery and Ask tabs. This binds
 * the one to the other: the pipeline segment vocabulary, which is derived run
 * state rather than a stored flag, is the only thing here that is not
 * common to the three.
 */
export function PipelineFilterBar({
  value,
  onChange,
  features,
  resultCount,
  className = '',
}: PipelineFilterBarProps): React.ReactElement {
  const counts = useMemo(() => segmentCounts(features), [features]);

  const segments = useMemo<readonly SegmentedOption<PipelineSegment>[]>(
    () => SEGMENTS.map((segment) => ({ ...segment, count: counts[segment.value] })),
    [counts],
  );

  const handleSort = useCallback(
    (sort: string) => {
      if (isPipelineSort(sort)) onChange({ ...value, sort });
    },
    [onChange, value],
  );

  return (
    <ListFilterBar
      testId="pipeline-filter-bar"
      noun="pipelines"
      segments={segments}
      segment={value.segment}
      onSegmentChange={(segment) => onChange({ ...value, segment })}
      query={value.query}
      onQueryChange={(query) => onChange({ ...value, query })}
      selects={[{ label: 'Sort pipelines', value: value.sort, options: SORTS, onChange: handleSort }]}
      narrowed={value.segment !== 'all' || value.query !== ''}
      onClear={() => onChange(clearFilters(value))}
      total={features.length}
      resultCount={resultCount}
      className={className}
    />
  );
}

const SEGMENTS: readonly SegmentedOption<PipelineSegment>[] = [
  { value: 'all', label: 'All' },
  // Amber is "a human is blocked" across the tree (runStatus.ts §F27); the
  // other segments take the control's own selected/idle colour so this one is
  // the only thing in the bar competing for attention.
  { value: 'needs-you', label: 'Needs you', countTone: 'amber' },
  { value: 'active', label: 'Active' },
  { value: 'done', label: 'Done' },
];

const SORTS: readonly { value: PipelineSort; label: string }[] = [
  { value: 'needs-you-first', label: 'Needs you first' },
  { value: 'newest', label: 'Newest first' },
  { value: 'oldest', label: 'Oldest first' },
];

function isPipelineSort(value: string): value is PipelineSort {
  return SORTS.some((sort) => sort.value === value);
}

export default PipelineFilterBar;
