/**
 * Which Discovery sessions and Ask threads a project's list shows, and in what
 * order.
 *
 * One module for both because they are the same row: a titled conversation
 * that is open or closed, run by a named harness, created once and touched
 * since. Splitting it would put the same four controls in front of the user
 * twice with two chances to disagree about what "oldest" means — the
 * coherence this exists to hold. Pipelines keep their own policy
 * ([`./pipelineFilter`]) because their segments are derived run states, not a
 * stored flag; what the three share is the *chrome*, in
 * `components/ui/ListFilterBar.tsx`.
 *
 * Three decisions worth keeping:
 *
 *  1. **`agentKind` is a stored string, not a union.** `AskThread.agent_kind`
 *     and `Discovery.agent_kind` are whatever the harness registry admitted,
 *     so a hand-written list here would drop a harness added elsewhere and
 *     silently filter its sessions to nothing. [`agentKinds`] derives the
 *     options from the rows instead.
 *
 *  2. **`recent` sorts on `updated_at`, `newest`/`oldest` on `created_at`.**
 *     They are different questions — "what was I last working on" against
 *     "what did I start when" — and a list that answered the first while
 *     labelled the second is why the distinction is spelled out rather than
 *     folded into one timestamp.
 *
 *  3. **Free text is a substring match**, for the reason
 *     [`./pipelineFilter`] records: a session title is prose, and the
 *     subsequence matcher `ProjectRail` uses over project *names* matches
 *     nearly every row of it.
 */

export type SessionSegment = 'all' | 'open' | 'closed';

/** The segments a session can actually be *in*: `'all'` is a filter, not a home. */
export type SessionStatus = Exclude<SessionSegment, 'all'>;

export type SessionSort = 'recent' | 'newest' | 'oldest';

/** What every agent-kind control means by "no choice made". Not a harness
 *  name, and not `''` — an empty `<option>` value reads as a real selection
 *  that happens to be blank. */
export const ANY_AGENT_KIND = 'all';

/** The fields this module reads off a Discovery or an Ask thread. */
export interface SessionRow {
  title: string;
  status: SessionStatus;
  agent_kind: string;
  created_at: number;
  updated_at: number;
}

export interface SessionFilterOptions {
  segment: SessionSegment;
  query: string;
  /** A harness name, or [`ANY_AGENT_KIND`]. */
  agentKind: string;
  sort: SessionSort;
}

export const DEFAULT_SESSION_FILTER: SessionFilterOptions = {
  segment: 'all',
  query: '',
  agentKind: ANY_AGENT_KIND,
  sort: 'recent',
};

/** Per-segment tallies for the control's count badges, over the *unqueried*
 *  list — a badge that shrank as you typed could not tell you what the other
 *  segments hold. Same rule as [`./pipelineFilter`]'s `segmentCounts`. */
export function segmentCounts(rows: readonly SessionRow[]): Record<SessionSegment, number> {
  const counts: Record<SessionSegment, number> = { all: rows.length, open: 0, closed: 0 };
  for (const row of rows) counts[row.status] += 1;
  return counts;
}

/** The harnesses actually present, in a stable order, so the dropdown never
 *  offers a choice that would match nothing. */
export function agentKinds(rows: readonly SessionRow[]): string[] {
  return [...new Set(rows.map((row) => row.agent_kind))].sort();
}

/** Whether any control that *hides* rows is set — what the bar's "matched
 *  nothing" notice and its reset are gated on. Sort is not one of them. */
export function isNarrowed(options: SessionFilterOptions): boolean {
  return (
    options.segment !== 'all' ||
    options.query !== '' ||
    options.agentKind !== ANY_AGENT_KIND
  );
}

/** Drop the choices that hide rows, keep the one that only reorders them —
 *  see [`./pipelineFilter`]'s `clearFilters` for why sort survives a reset.
 *
 *  Returns `options` itself when nothing was narrowed. */
export function clearFilters(options: SessionFilterOptions): SessionFilterOptions {
  if (!isNarrowed(options)) return options;
  return { ...options, segment: 'all', query: '', agentKind: ANY_AGENT_KIND };
}

function matchesQuery(row: SessionRow, terms: readonly string[]): boolean {
  if (terms.length === 0) return true;
  const haystack = row.title.toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/**
 * Filter and order a session list.
 *
 * Returns `rows` itself when nothing was dropped and nothing moved, so a
 * memoized consumer re-renders only when the visible list genuinely changed.
 */
export function filterSessions<T extends SessionRow>(
  rows: T[],
  options: SessionFilterOptions,
): T[] {
  const terms = options.query.trim().toLowerCase().split(/\s+/).filter((t) => t.length > 0);

  const kept: T[] = [];
  for (const row of rows) {
    if (options.segment !== 'all' && row.status !== options.segment) continue;
    if (options.agentKind !== ANY_AGENT_KIND && row.agent_kind !== options.agentKind) continue;
    if (!matchesQuery(row, terms)) continue;
    kept.push(row);
  }

  // The index tiebreak is what makes the order stable for equal keys — the
  // requirement is the contract, not an engine detail to rely on.
  const ordered = kept
    .map((row, index) => ({ row, index }))
    .sort((a, b) => {
      const age =
        options.sort === 'recent'
          ? b.row.updated_at - a.row.updated_at
          : options.sort === 'oldest'
            ? a.row.created_at - b.row.created_at
            : b.row.created_at - a.row.created_at;
      if (age !== 0) return age;
      return a.index - b.index;
    })
    .map((entry) => entry.row);

  if (ordered.length === rows.length && ordered.every((row, index) => row === rows[index])) {
    return rows;
  }

  return ordered;
}
