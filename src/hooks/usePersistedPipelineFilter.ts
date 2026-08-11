import { useCallback, useMemo, useState } from 'react';

import { DEFAULT_PIPELINE_FILTER, type PipelineFilterOptions } from '../lib/pipelineFilter';
import { pipelineSegmentPref, pipelineSortPref } from '../lib/uiPrefs';
import { usePersistedPref } from './usePersistedPref';

/**
 * The project view's filter, with the two structural halves of
 * `PipelineFilterOptions` surviving a restart (UI_REDESIGN_PLAN §6 Phase 6).
 *
 * The value is memoized because `ProjectHome` feeds it to the `useMemo` around
 * `filterPipelines`, which exists to hold across a keystroke elsewhere in that
 * component.
 */
export function usePersistedPipelineFilter(): [
  PipelineFilterOptions,
  (next: PipelineFilterOptions) => void,
] {
  const [segment, chooseSegment] = usePersistedPref(
    pipelineSegmentPref,
    DEFAULT_PIPELINE_FILTER.segment,
  );
  const [sort, chooseSort] = usePersistedPref(pipelineSortPref, DEFAULT_PIPELINE_FILTER.sort);
  /* No `UiPref` for the query, which `uiPrefs.ts` records as a decision rather
     than a gap left here to close. Everything else is stored as it stands,
     `clearFilters` included — the reset writes `segment: 'all'` because that is
     then the structural state. */
  const [query, setQuery] = useState(DEFAULT_PIPELINE_FILTER.query);

  const value = useMemo<PipelineFilterOptions>(
    () => ({ segment, query, sort }),
    [segment, query, sort],
  );

  // `PipelineFilterBar` hands back the whole object whichever control moved, so
  // an unguarded forward would spend a stored round trip per keystroke on a
  // segment and a sort that never changed.
  const choose = useCallback(
    (next: PipelineFilterOptions) => {
      if (next.segment !== segment) chooseSegment(next.segment);
      if (next.sort !== sort) chooseSort(next.sort);
      setQuery(next.query);
    },
    [chooseSegment, chooseSort, segment, sort],
  );

  return [value, choose];
}
