import React, { useMemo } from 'react';

import {
  agentKinds,
  clearFilters,
  isNarrowed,
  segmentCounts,
  ANY_AGENT_KIND,
  type SessionFilterOptions,
  type SessionRow,
  type SessionSegment,
  type SessionSort,
} from '../lib/sessionFilter';
import { ListFilterBar } from './ui/ListFilterBar';
import type { SegmentedOption } from './ui/SegmentedControl';

export interface SessionFilterBarProps {
  value: SessionFilterOptions;
  onChange: (next: SessionFilterOptions) => void;
  /** The whole unfiltered list — both `segmentCounts` and the agent-kind
   *  options are deliberately taken over it, so neither a badge nor a
   *  dropdown entry disappears because of a choice already made. */
  sessions: readonly SessionRow[];
  /** What `filterSessions` kept, so the bar can own the empty state its own
   *  controls caused without recomputing the filter. */
  resultCount: number;
  /** Plural, lower case — "discoveries" or "threads". */
  noun: string;
  testId?: string;
  className?: string;
}

/**
 * The Discovery and Ask tabs' filter bar: status + query + harness + order.
 *
 * One component for two tabs, over one policy (`lib/sessionFilter.ts`), for
 * the reason that module records — the two lists are the same row, so two
 * bars would be two chances to disagree about what "oldest" means. Pipelines
 * reach the same chrome through `PipelineFilterBar` with their own segments.
 */
export function SessionFilterBar({
  value,
  onChange,
  sessions,
  resultCount,
  noun,
  testId = 'session-filter-bar',
  className = '',
}: SessionFilterBarProps): React.ReactElement {
  const counts = useMemo(() => segmentCounts(sessions), [sessions]);

  const segments = useMemo<readonly SegmentedOption<SessionSegment>[]>(
    () => SEGMENTS.map((segment) => ({ ...segment, count: counts[segment.value] })),
    [counts],
  );

  // Rendered only when the project has run more than one harness: a dropdown
  // whose every choice keeps every row is chrome that answers nothing.
  const kinds = useMemo(() => agentKinds(sessions), [sessions]);

  const selects = useMemo(() => {
    const sort = {
      label: `Sort ${noun}`,
      value: value.sort,
      options: SORTS,
      onChange: (next: string) => {
        if (isSessionSort(next)) onChange({ ...value, sort: next });
      },
    };
    if (kinds.length < 2) return [sort];
    return [
      {
        label: 'Filter by agent',
        value: value.agentKind,
        options: [
          { value: ANY_AGENT_KIND, label: 'Any agent' },
          ...kinds.map((kind) => ({ value: kind, label: kind })),
        ],
        onChange: (agentKind: string) => onChange({ ...value, agentKind }),
      },
      sort,
    ];
  }, [kinds, noun, onChange, value]);

  return (
    <ListFilterBar
      testId={testId}
      noun={noun}
      segments={segments}
      segment={value.segment}
      onSegmentChange={(segment) => onChange({ ...value, segment })}
      query={value.query}
      onQueryChange={(query) => onChange({ ...value, query })}
      selects={selects}
      narrowed={isNarrowed(value)}
      onClear={() => onChange(clearFilters(value))}
      total={sessions.length}
      resultCount={resultCount}
      className={className}
    />
  );
}

const SEGMENTS: readonly SegmentedOption<SessionSegment>[] = [
  { value: 'all', label: 'All' },
  { value: 'open', label: 'Open' },
  { value: 'closed', label: 'Closed' },
];

const SORTS: readonly { value: SessionSort; label: string }[] = [
  { value: 'recent', label: 'Recently touched' },
  { value: 'newest', label: 'Newest first' },
  { value: 'oldest', label: 'Oldest first' },
];

function isSessionSort(value: string): value is SessionSort {
  return SORTS.some((sort) => sort.value === value);
}

export default SessionFilterBar;
