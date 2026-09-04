/**
 * The bounded fold of a run's durable event log, shared by both transports:
 * `useRunEvents` merges the local Tauri push plus its backfill, `useRemoteRun`
 * merges the batches the detached tail polls off the runner. They saw the same
 * log through two hand-written folds that had drifted — one sorted by offset
 * and one did not, one rebuilt its de-dupe set from the already-truncated feed
 * and so could re-append a row it had evicted — which is a divergence a user
 * can read off the Activity strip while §2 says the two must render alike.
 *
 * Two properties the callers depend on and neither can restore afterwards:
 *
 *  - **Assignments fold before the cap.** `agent_spawned` evidence outlives the
 *    row that carried it; a long run pushes that row out of the window while
 *    the step it describes is still on screen.
 *  - **Reconciliation is by durable offset, not arrival order**, so the
 *    backfill and the live push are safe to interleave in either order.
 *
 * De-dupe is bounded to match: `seenOffsets` covers only the retained window,
 * and everything the cap has already evicted is refused by `evictedThrough`
 * instead of by a set that would grow for the lifetime of the view. Offsets
 * are monotonic per log, so the two together are exact for every row that has
 * been delivered — and a row *below* the window that was never delivered is
 * one the cap would drop again on the next merge.
 */
import {
  reconcileRunEventAssignments,
  type RunEventAssignments,
} from './runEventAssignments';
import type { RunEvent } from '../types';

/** Rows retained for the Activity strip. A long run emits thousands; the
 *  strip shows a recent window, and assignments survive the cut above. */
export const MAX_FEED_EVENTS = 500;

export interface RunEventFeed {
  /** Oldest→newest by durable offset, capped at [`MAX_FEED_EVENTS`]. */
  events: RunEvent[];
  seenOffsets: ReadonlySet<number>;
  /** Highest offset the cap has dropped; nothing at or below it is re-taken. */
  evictedThrough: number;
  assignments: RunEventAssignments;
}

export const EMPTY_ASSIGNMENTS: RunEventAssignments = {};
const NO_EVENTS: RunEvent[] = [];
const NO_OFFSETS: ReadonlySet<number> = new Set();

/** A feed holding nothing — a constant, so a hook that re-derives it on a
 *  render does not hand its consumers a new `assignments` identity. */
export const EMPTY_RUN_EVENT_FEED: RunEventFeed = {
  events: NO_EVENTS,
  seenOffsets: NO_OFFSETS,
  evictedThrough: Number.NEGATIVE_INFINITY,
  assignments: EMPTY_ASSIGNMENTS,
};

/**
 * Fold `incoming` into `feed`, returning `feed` itself when nothing was new so
 * a caller storing this in React state re-renders only on a real change.
 */
export function mergeRunEventFeed(
  feed: RunEventFeed,
  incoming: readonly RunEvent[],
): RunEventFeed {
  const accepted: RunEvent[] = [];
  const taken = new Set<number>();
  for (const event of incoming) {
    if (event.offset <= feed.evictedThrough) continue;
    if (feed.seenOffsets.has(event.offset) || taken.has(event.offset)) continue;
    taken.add(event.offset);
    accepted.push(event);
  }
  if (accepted.length === 0) return feed;

  const assignments = reconcileRunEventAssignments(feed.assignments, accepted);
  const merged = [...feed.events, ...accepted].sort((a, b) => a.offset - b.offset);
  const overflow = Math.max(merged.length - MAX_FEED_EVENTS, 0);
  const events = overflow > 0 ? merged.slice(overflow) : merged;

  return {
    events,
    seenOffsets: new Set(events.map((event) => event.offset)),
    evictedThrough:
      overflow > 0
        ? Math.max(feed.evictedThrough, merged[overflow - 1].offset)
        : feed.evictedThrough,
    assignments,
  };
}
