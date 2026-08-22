import { useEffect, useMemo, useState } from 'react';
import type { NodeRunStatus } from '../components/canvas/types';
import { listRunEventsSince } from '../lib/featureDetail';
import {
  reconcileRunEventAssignments,
  type RunEventAssignments,
} from '../lib/runEventAssignments';
import type { RunEvent, StepExecution } from '../types';
import { useTauriEvent } from './useTauriEvent';

const MAX_EVENTS = 500;

interface RetryDecision {
  errorClass: string;
  offset: number;
}

interface FeatureRunEventState {
  featureId: string;
  events: RunEvent[];
  seenOffsets: Set<number>;
  assignments: RunEventAssignments;
  retryDecisions: Record<string, RetryDecision>;
}

export interface RunEventsState {
  statusByNode: Record<string, NodeRunStatus>;
  events: RunEvent[];
  assignments: RunEventAssignments;
}

function emptyState(featureId: string): FeatureRunEventState {
  return {
    featureId,
    events: [],
    seenOffsets: new Set(),
    assignments: {},
    retryDecisions: {},
  };
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
  const seenOffsets = new Set(state.seenOffsets);
  const accepted: RunEvent[] = [];

  for (const event of incoming) {
    if (event.run_id !== state.featureId || seenOffsets.has(event.offset)) continue;
    seenOffsets.add(event.offset);
    accepted.push(event);
  }

  if (accepted.length === 0) return state;

  const retryDecisions = { ...state.retryDecisions };
  for (const event of accepted) {
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

  const events = [...state.events, ...accepted].sort((a, b) => a.offset - b.offset);

  return {
    ...state,
    events: events.length > MAX_EVENTS ? events.slice(events.length - MAX_EVENTS) : events,
    seenOffsets,
    assignments: reconcileRunEventAssignments(state.assignments, accepted),
    retryDecisions,
  };
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

  const scopedState = state.featureId === featureId ? state : emptyState(featureId);
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
      const assignment = scopedState.assignments[step.id];
      map[step.step_id] = {
        status: step.status,
        costUsd: step.cost_usd ?? null,
        wallClockSecs: step.wall_clock_secs ?? null,
        tokens: step.tokens ?? null,
        errorClass: scopedState.retryDecisions[step.step_id]?.errorClass ?? null,
        stepExecutionId: step.id,
        ...(assignment
          ? { agentKind: assignment.agentKind, effort: assignment.effort }
          : {}),
      };
    }
    return map;
  }, [steps, scopedState.assignments, scopedState.retryDecisions]);

  return {
    statusByNode,
    events: scopedState.events,
    assignments: scopedState.assignments,
  };
}
