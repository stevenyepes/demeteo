import { Search, X } from 'lucide-react';
import React, { useCallback, useMemo } from 'react';

import {
  clearFilters,
  segmentCounts,
  type PipelineFilterOptions,
  type PipelineRow,
  type PipelineSegment,
  type PipelineSort,
} from '../lib/pipelineFilter';
import { SegmentedControl, type SegmentedOption } from './ui/SegmentedControl';

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
 * Every rule about *which* rows survive lives in `lib/pipelineFilter.ts`; this
 * only binds three controls to `PipelineFilterOptions` (§5.2). Three choices
 * that are not recoverable from the markup:
 *
 * **Sort is a `<select>`, not a second `SegmentedControl`.** Two radiogroups in
 * one bar both paint their selection in `TONE_CHIP.cyan`, so the segment row —
 * which carries the needs-you promise and is scanned on every visit — would
 * compete with a control most users set once. The native control also collapses
 * three options into one line of chrome and arrives keyboard- and AT-complete,
 * matching the repository picker already in this header.
 *
 * **The bar owns "your filter matched nothing"; `ProjectHome` owns "this
 * project has no features".** The first is a state the user caused, so the undo
 * belongs beside the controls that caused it; the second is not a filter
 * outcome and a reset would not help it. The bar tells them apart from
 * `features` being empty, so it can be rendered unconditionally.
 *
 * **No debounce.** `filterPipelines` returns the input array's identity when
 * nothing changed, which is what keeps a memoized list cheap per keystroke; a
 * debounce here would buy nothing and cost the input its immediacy. If a
 * profile ever says otherwise, that measurement is the argument, not this.
 */
export function PipelineFilterBar({
  value,
  onChange,
  features,
  resultCount,
  className = '',
}: PipelineFilterBarProps): React.ReactElement {
  const counts = useMemo(() => segmentCounts(features), [features]);

  const options = useMemo<readonly SegmentedOption<PipelineSegment>[]>(
    () => SEGMENTS.map((segment) => ({ ...segment, count: counts[segment.value] })),
    [counts],
  );

  const handleSegment = useCallback(
    (segment: PipelineSegment) => onChange({ ...value, segment }),
    [onChange, value],
  );

  const handleQuery = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => onChange({ ...value, query: event.target.value }),
    [onChange, value],
  );

  const handleClearQuery = useCallback(() => onChange({ ...value, query: '' }), [onChange, value]);

  const handleSort = useCallback(
    (event: React.ChangeEvent<HTMLSelectElement>) => {
      const sort = event.target.value;
      if (isPipelineSort(sort)) onChange({ ...value, sort });
    },
    [onChange, value],
  );

  const handleClearFilters = useCallback(() => onChange(clearFilters(value)), [onChange, value]);

  return (
    <div className={`flex flex-col gap-2 ${className}`} data-testid="pipeline-filter-bar">
      <div className="flex flex-wrap items-center gap-2">
        <SegmentedControl
          options={options}
          value={value.segment}
          onChange={handleSegment}
          ariaLabel="Filter pipelines"
          size="sm"
        />

        <div className="relative min-w-0 flex-1 sm:max-w-xs">
          <Search
            className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500"
            aria-hidden
          />
          {/* Escape is not bound to "clear": `useKeyboardShortcuts` owns it
              globally and unguarded, so a second meaning here would fire the
              app-level handler on the same press. */}
          <input
            type="search"
            value={value.query}
            onChange={handleQuery}
            placeholder="Filter pipelines..."
            aria-label="Filter pipelines by text"
            className="w-full rounded-md border border-white/5 bg-black/30 py-1.5 pl-7 pr-7 text-[11px] text-white placeholder-slate-600 focus:border-cyan-500/30 focus:outline-none"
          />
          {value.query !== '' && (
            <button
              type="button"
              onClick={handleClearQuery}
              aria-label="Clear the filter text"
              className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-slate-500 transition-colors hover:bg-white/5 hover:text-slate-200"
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </div>

        <select
          value={value.sort}
          onChange={handleSort}
          aria-label="Sort pipelines"
          className="shrink-0 rounded-md border border-white/10 bg-black/20 px-2 py-1.5 text-[11px] font-mono text-slate-300 outline-none hover:border-white/20 focus:border-cyan-500/50"
        >
          {SORTS.map((sort) => (
            <option key={sort.value} value={sort.value}>
              {sort.label}
            </option>
          ))}
        </select>
      </div>

      {features.length > 0 && resultCount === 0 && (
        <p role="status" className="flex items-center gap-2 text-[11px] text-slate-500">
          No pipelines match this filter.
          <button
            type="button"
            onClick={handleClearFilters}
            className="font-medium text-cyan-400 transition-colors hover:text-cyan-300"
          >
            Clear filters
          </button>
        </p>
      )}
    </div>
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
