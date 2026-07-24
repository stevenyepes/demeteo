import { useMemo, useState } from 'react';
import { useTauriEvent } from './useTauriEvent';
import type { RunEvent, StepExecution } from '../types';
import type { NodeRunStatus } from '../components/canvas/types';

/**
 * The single run-event consumer both run-mode surfaces share (P2.2): the
 * workflow **canvas** overlay and the **timeline** read node status from the
 * same place, so they can never disagree about what a run is doing.
 *
 * Two inputs, one shape out:
 *  - `steps` (the feature's `step_executions` snapshot, reloaded by
 *    `FeatureDetail` on every `step_progress`/`feature_status_changed` event)
 *    is the authoritative status/cost/duration source. Using it — rather than
 *    replaying `run_event` deltas from zero — means the overlay is correct on
 *    first mount and after a full reload, with no gap-recovery bookkeeping.
 *  - the unified `run_events` stream (P1.13) supplies the one thing a step row
 *    can't: the **failure class** (`retry_decision`), lifted per node so a
 *    failed card can name *why* it failed. The raw feed is also returned for
 *    the panel's Overview tab (P2.3) and the remote path (P2.6).
 *
 * Node ids are the migrated v2 node ids, which equal the v1 `step_id` the
 * migration preserves — so `statusByNode[node.id]` resolves directly.
 */

/** Cap the retained raw feed — a long run can emit thousands of rows and the
 *  panel only needs a recent window. */
const MAX_EVENTS = 500;

export interface RunEventsState {
  /** node id (== `step_id`) → live run state for the canvas overlay. */
  statusByNode: Record<string, NodeRunStatus>;
  /** Raw append-only run-event feed (P1.13), oldest→newest, bounded. */
  events: RunEvent[];
}

export function useRunEvents(
  featureId: string,
  steps: StepExecution[],
): RunEventsState {
  const [events, setEvents] = useState<RunEvent[]>([]);
  // Failure class per node id: the `step_executions` row records that a step
  // failed, not which class it was — that lives only in the `retry_decision`
  // run-event (P1.10). Accumulated here across reloads within this run's view.
  const [errorClassByNode, setErrorClassByNode] = useState<Record<string, string>>({});

  useTauriEvent<RunEvent>(
    'run_event',
    (evt) => {
      // Local runs key the log by feature id (a local run has no runner run
      // row — the feature id *is* its run id, per P1.13). Drop other runs.
      if (evt.run_id !== featureId) return;

      setEvents((prev) => {
        const next = [...prev, evt];
        return next.length > MAX_EVENTS ? next.slice(next.length - MAX_EVENTS) : next;
      });

      if (evt.kind === 'retry_decision' && evt.payload_json) {
        try {
          const p = JSON.parse(evt.payload_json);
          if (typeof p?.step_id === 'string' && typeof p?.error_class === 'string') {
            setErrorClassByNode((prev) =>
              prev[p.step_id] === p.error_class
                ? prev
                : { ...prev, [p.step_id]: p.error_class },
            );
          }
        } catch {
          /* malformed payload — skip */
        }
      }
    },
    [featureId],
  );

  const statusByNode = useMemo(() => {
    const map: Record<string, NodeRunStatus> = {};
    const seenUpdated: Record<string, number> = {};
    for (const s of steps) {
      // One node id can back several executions (replay/retry produce new
      // rows); keep the most recently updated so the overlay tracks the live
      // attempt rather than a stale one.
      if (s.step_id in seenUpdated && seenUpdated[s.step_id] >= s.updated_at) continue;
      seenUpdated[s.step_id] = s.updated_at;
      map[s.step_id] = {
        status: s.status,
        costUsd: s.cost_usd ?? null,
        wallClockSecs: s.wall_clock_secs ?? null,
        tokens: s.tokens ?? null,
        errorClass: errorClassByNode[s.step_id] ?? null,
        stepExecutionId: s.id,
      };
    }
    return map;
  }, [steps, errorClassByNode]);

  return { statusByNode, events };
}
