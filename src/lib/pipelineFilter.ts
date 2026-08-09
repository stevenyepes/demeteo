/**
 * Which pipelines the project view shows, and in what order.
 *
 * The list is every non-archived feature, so a gate — the one thing in the app
 * that cannot progress without the user — arrives interleaved with finished and
 * failed runs in creation order. Segmenting and banding it is the fix
 * (`docs/UI_REDESIGN_PLAN.md` §3.2); the policy lives here rather than in
 * `ProjectHome` because it decides *what should be shown*, reads no DOM, and is
 * answerable from a test with a handful of plain objects (§5.2).
 *
 * Three decisions worth keeping:
 *
 *  1. **Membership is derived from `runStatusMeta`, never re-listed.** A second
 *     hand-written table of status strings is the drift F27 already paid for
 *     once. Adding a status to `runStatus.ts` places it here automatically.
 *
 *  2. **"Needs you" is amber *and* not moving.** Amber alone is not enough:
 *     `bootstrapping` is amber while it is still working on its own, and a run
 *     nobody is blocked on must not outrank a real gate. A human decision is
 *     pending precisely when the run has stopped and stayed amber.
 *
 *  3. **Free text is a substring match, not the subsequence matcher in
 *     `ProjectRail`.** That one exists for project *names* — a dozen
 *     characters, where a subsequence hit is almost always intentional. Run
 *     descriptions are prose, and over prose a three-letter query matches
 *     nearly every row, which reads as a filter that does nothing.
 */

import { featureRunStatus, runStatusMeta, type FeatureRunStatusFields } from './runStatus';

export type PipelineSegment = 'all' | 'needs-you' | 'active' | 'done';

/** The segments a feature can actually be *in*: `'all'` is a filter, not a home. */
export type PipelineBand = Exclude<PipelineSegment, 'all'>;

export type PipelineSort = 'needs-you-first' | 'newest' | 'oldest';

/** The fields this module reads off a feature row. */
export interface PipelineRow extends FeatureRunStatusFields {
  title: string;
  description?: string | null;
  created_at: number;
}

export interface PipelineFilterOptions {
  segment: PipelineSegment;
  query: string;
  sort: PipelineSort;
}

export const DEFAULT_PIPELINE_FILTER: PipelineFilterOptions = {
  segment: 'all',
  query: '',
  sort: 'needs-you-first',
};

export function segmentFor(feature: PipelineRow): PipelineBand {
  const meta = runStatusMeta(featureRunStatus(feature));
  if (meta.tone === 'amber' && !meta.active) return 'needs-you';
  if (meta.active) return 'active';
  return 'done';
}

/** Per-segment tallies for the filter control's count badges.
 *
 *  Here rather than in the component because the alternative is
 *  `features.filter(f => segmentFor(f) === 'needs-you').length` spelled in a
 *  render — policy in a render, and a fourth place that would have to agree
 *  with [`segmentFor`]. Counts are of the *unqueried* list: a badge that shrank
 *  as you typed could not tell you what the other segments hold. */
export function segmentCounts(features: readonly PipelineRow[]): Record<PipelineSegment, number> {
  const counts: Record<PipelineSegment, number> = {
    all: features.length,
    'needs-you': 0,
    active: 0,
    done: 0,
  };
  for (const feature of features) counts[segmentFor(feature)] += 1;
  return counts;
}

const BAND_RANK: Record<PipelineBand, number> = {
  'needs-you': 0,
  active: 1,
  done: 2,
};

function matchesQuery(feature: PipelineRow, terms: readonly string[]): boolean {
  if (terms.length === 0) return true;
  const haystack = `${feature.title}\n${feature.description ?? ''}`.toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/**
 * Filter and order the pipeline list.
 *
 * Returns `features` itself when nothing was dropped and nothing moved, so a
 * memoized consumer re-renders only when the visible list genuinely changed.
 */
export function filterPipelines<T extends PipelineRow>(
  features: T[],
  options: PipelineFilterOptions,
): T[] {
  const terms = options.query.trim().toLowerCase().split(/\s+/).filter((t) => t.length > 0);

  const kept: T[] = [];
  for (const feature of features) {
    if (options.segment !== 'all' && segmentFor(feature) !== options.segment) continue;
    if (!matchesQuery(feature, terms)) continue;
    kept.push(feature);
  }

  // The index tiebreak is what makes the order stable for equal keys — the
  // requirement is the contract, not an engine detail to rely on.
  const ordered = kept
    .map((feature, index) => ({ feature, index }))
    .sort((a, b) => {
      if (options.sort === 'needs-you-first') {
        const band = BAND_RANK[segmentFor(a.feature)] - BAND_RANK[segmentFor(b.feature)];
        if (band !== 0) return band;
      }
      const age = options.sort === 'oldest'
        ? a.feature.created_at - b.feature.created_at
        : b.feature.created_at - a.feature.created_at;
      if (age !== 0) return age;
      return a.index - b.index;
    })
    .map((entry) => entry.feature);

  if (
    ordered.length === features.length &&
    ordered.every((feature, index) => feature === features[index])
  ) {
    return features;
  }

  return ordered;
}
