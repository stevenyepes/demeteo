/**
 * The single run-event consumer both run-mode surfaces share (P2.2): the
 * workflow **canvas** overlay and the **timeline** read node status from the
 * same place, so they can never disagree about what a run is doing.
 *
 * Two inputs, one shape out:
 *  - `steps` (the feature's `step_executions` snapshot, reloaded by
 *    `FeatureDetail` on every `step_progress`/`feature_status_changed` event)
 *    is the authoritative status/cost/duration source. Deltas are never
 *    replayed to derive those: a step row already holds the settled value, so
 *    reading it keeps the overlay correct after a reload with no gap-recovery
 *    bookkeeping. The log is read for what a step row cannot carry, and only
 *    for that.
 *  - the unified `run_events` stream (P1.13) supplies two such things. The
 *    **failure class** (`retry_decision`) is lifted per node so a failed card
 *    can name *why* it failed — `step_executions` has no column for it, and
 *    the rule that chose it is not recoverable from the status alone. The
 *    **spawn evidence** (`agent_spawned`) is what the run actually launched,
 *    which the workflow definition can only predict. Both are backfilled from
 *    offset 0 on mount, because both outlive the push subscription: a view
 *    opened after the fact would otherwise show a blank where the record is.
 *
 * Node ids are the migrated v2 node ids, which equal the v1 `step_id` the
 * migration preserves — so `statusByNode[node.id]` resolves directly.
 *
 * Assignments come out keyed by `step_execution_id` and are deliberately *not*
 * projected onto nodes here: `statusByNode` keeps one execution per node id,
 * and the timeline renders every attempt. Joining the two is the caller's, in
 * `useRunGraph`, which is also where a detached run substitutes its own
 * evidence — merging it in here would produce a value that caller can only
 * discard.
 */
import { useEffect, useMemo, useState } from 'react';
import type { NodeRunStatus } from '../components/canvas/types';
import { listRunEventsSince } from '../lib/featureDetail';
import type { RunEventAssignments } from '../lib/runEventAssignments';
import {
  EMPTY_RUN_EVENT_FEED,
  mergeRunEventFeed,
  type RunEventFeed,
} from '../lib/runEventFeed';
import type { RunEvent, StepExecution } from '../types';
import { useTauriEvent } from './useTauriEvent';

interface RetryDecision {
  errorClass: string;
  offset: number;
}

const NO_RETRY_DECISIONS: Record<string, RetryDecision> = {};

interface FeatureRunEventState {
  featureId: string;
  feed: RunEventFeed;
  retryDecisions: Record<string, RetryDecision>;
}

function emptyState(featureId: string): FeatureRunEventState {
  return {
    featureId,
    feed: EMPTY_RUN_EVENT_FEED,
    retryDecisions: NO_RETRY_DECISIONS,
  };
}

export interface RunEventsState {
  /** node id (== `step_id`) → live run state for the canvas overlay. */
  statusByNode: Record<string, NodeRunStatus>;
  /** Raw append-only run-event feed (P1.13), oldest→newest, bounded. */
  events: RunEvent[];
  /** Newest spawn evidence per `step_execution_id`, unbounded by the feed cap. */
  assignments: RunEventAssignments;
}

function retryDecision(event: RunEvent): { stepId: string; errorClass: string } | null {
  if (event.kind !== 'retry_decision' || !event.payload_json) return null;

  try {
    const payload: unknown = JSON.parse(event.payload_json);
    if (typeof payload !== 'object' || payload === null) return null;
    const candidate = payload as Record<string, unknown>;
    if (typeof candidate.step_id !== 'string' || typeof candidate.error_class !== 'string') {
      return null;
    }
    return { stepId: candidate.step_id, errorClass: candidate.error_class };
  } catch {
    return null;
  }
}

function mergeEvents(
  state: FeatureRunEventState,
  incoming: readonly RunEvent[],
): FeatureRunEventState {
  const scoped = incoming.filter((event) => event.run_id === state.featureId);
  const feed = mergeRunEventFeed(state.feed, scoped);
  if (feed === state.feed) return state;

  const retryDecisions = { ...state.retryDecisions };
  for (const event of scoped) {
    const decision = retryDecision(event);
    if (!decision) continue;
    const existing = retryDecisions[decision.stepId];
    if (!existing || existing.offset < event.offset) {
      retryDecisions[decision.stepId] = {
        errorClass: decision.errorClass,
        offset: event.offset,
      };
    }
  }

  return { ...state, feed, retryDecisions };
}

export function useRunEvents(
  featureId: string,
  steps: StepExecution[],
): RunEventsState {
  const [state, setState] = useState<FeatureRunEventState>(() => emptyState(featureId));

  useEffect(() => {
    let active = true;
    setState((current) =>
      current.featureId === featureId ? current : emptyState(featureId),
    );

    void listRunEventsSince(featureId, 0)
      .then((history) => {
        if (!active) return;
        setState((current) => {
          const scoped = current.featureId === featureId ? current : emptyState(featureId);
          return mergeEvents(scoped, history);
        });
      })
      .catch(() => undefined);

    return () => {
      active = false;
    };
  }, [featureId]);

  useTauriEvent<RunEvent>(
    'run_event',
    (event) => {
      if (event.run_id !== featureId) return;
      setState((current) => {
        const scoped = current.featureId === featureId ? current : emptyState(featureId);
        return mergeEvents(scoped, [event]);
      });
    },
    [featureId],
  );

  // The render between a `featureId` change and the effect that scopes state to
  // it still holds the previous feature's fold; the constants make that render
  // hand out the same empty identities every time rather than fresh ones the
  // memo below would have to re-run for.
  const scoped = state.featureId === featureId ? state : emptyState(featureId);
  const { feed, retryDecisions } = scoped;
  const statusByNode = useMemo(() => {
    const map: Record<string, NodeRunStatus> = {};
    const seenUpdated: Record<string, number> = {};
    for (const step of steps) {
      if (
        step.step_id in seenUpdated &&
        seenUpdated[step.step_id] >= step.updated_at
      ) {
        continue;
      }
      seenUpdated[step.step_id] = step.updated_at;
      map[step.step_id] = {
        status: step.status,
        costUsd: step.cost_usd ?? null,
        wallClockSecs: step.wall_clock_secs ?? null,
        tokens: step.tokens ?? null,
        errorClass: retryDecisions[step.step_id]?.errorClass ?? null,
        stepExecutionId: step.id,
      };
    }
    return map;
  }, [steps, retryDecisions]);

  return {
    statusByNode,
    events: feed.events,
    assignments: feed.assignments,
  };
}
