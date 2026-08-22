/**
 * `useRunEvents` (P2.2): the single overlay derivation both run-mode surfaces
 * share. These prove the two inputs fold into one `statusByNode` shape — the
 * authoritative `step_executions` snapshot for status/cost/duration, and the
 * `run_events` stream for the failure class a step row can't carry — and that
 * events for other runs are ignored.
 */
import { renderHook, act, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { listRunEventsSince } = vi.hoisted(() => ({
  listRunEventsSince: vi.fn<() => Promise<RunEvent[]>>(),
}));

vi.mock('../lib/featureDetail', () => ({ listRunEventsSince }));

// Capture `listen` handlers so a test can dispatch synthetic Tauri events.
const handlers: Record<string, Array<(e: { payload: unknown }) => void>> = {};
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => {
    (handlers[event] ??= []).push(cb);
    return Promise.resolve(() => {
      handlers[event] = (handlers[event] ?? []).filter((h) => h !== cb);
    });
  },
}));

import { useRunEvents } from './useRunEvents';
import type { RunEvent, StepExecution } from '../types';

function emit(event: string, payload: unknown) {
  for (const h of handlers[event] ?? []) h({ payload });
}

const step = (over: Partial<StepExecution>): StepExecution => ({
  id: 'se-1',
  feature_id: 'f1',
  step_id: 'research',
  step_index: 0,
  step_kind: 'agent',
  status: 'completed',
  artifact_paths: [],
  created_at: 0,
  updated_at: 1,
  ...over,
});

const runEvent = (over: Partial<RunEvent>): RunEvent => ({
  offset: 1,
  run_id: 'f1',
  kind: 'step_progress',
  payload_json: null,
  created_at: 0,
  ...over,
});

const spawnedEvent = (
  offset: number,
  stepExecutionId: string,
  agentKind: string,
  effort: 'low' | 'medium' | 'high' | 'xhigh' | 'max' | null,
): RunEvent =>
  runEvent({
    offset,
    kind: 'agent_spawned',
    payload_json: JSON.stringify({
      step_execution_id: stepExecutionId,
      agent_kind: agentKind,
      effort,
    }),
  });

afterEach(() => {
  for (const k of Object.keys(handlers)) delete handlers[k];
  listRunEventsSince.mockReset();
});

beforeEach(() => {
  listRunEventsSince.mockResolvedValue([]);
});

describe('useRunEvents', () => {
  it('backfills from zero and merges a newer live event before history resolves', async () => {
    let resolveHistory: (events: RunEvent[]) => void = () => undefined;
    listRunEventsSince.mockReturnValue(
      new Promise((resolve) => {
        resolveHistory = resolve;
      }),
    );
    const steps = [step({ id: 'se-1', step_id: 'implement' })];
    const { result } = renderHook(() => useRunEvents('f1', steps));

    await waitFor(() => expect(listRunEventsSince).toHaveBeenCalledWith('f1', 0));
    await waitFor(() => expect(handlers.run_event?.length).toBeGreaterThan(0));

    act(() => emit('run_event', spawnedEvent(3, 'se-1', 'claude-code', 'xhigh')));
    act(() => {
      resolveHistory([
        spawnedEvent(1, 'se-1', 'opencode', 'high'),
        runEvent({ offset: 2 }),
        spawnedEvent(3, 'se-1', 'stale-duplicate', 'low'),
        runEvent({ offset: 4, run_id: 'other-run' }),
      ]);
    });

    await waitFor(() => expect(result.current.events.map((event) => event.offset)).toEqual([1, 2, 3]));
    expect(result.current.statusByNode.implement).toMatchObject({
      stepExecutionId: 'se-1',
      agentKind: 'claude-code',
      effort: 'xhigh',
    });
  });

  it('retains assignments after their event leaves the bounded feed', async () => {
    listRunEventsSince.mockResolvedValue([spawnedEvent(1, 'se-1', 'hermes', null)]);
    const laterEvents = Array.from({ length: 501 }, (_, index) =>
      runEvent({ offset: index + 2 }),
    );
    const { result } = renderHook(() =>
      useRunEvents('f1', [step({ id: 'se-1', step_id: 'implement' })]),
    );

    await waitFor(() => expect(result.current.events).toHaveLength(1));
    act(() => {
      for (const event of laterEvents) emit('run_event', event);
    });

    await waitFor(() => expect(result.current.events).toHaveLength(500));
    expect(result.current.events[0]?.offset).toBe(3);
    expect(result.current.statusByNode.implement).toMatchObject({
      agentKind: 'hermes',
      effort: null,
    });
  });

  it('enriches only the newest execution selected for a node', async () => {
    listRunEventsSince.mockResolvedValue([
      spawnedEvent(1, 'se-old', 'opencode', 'max'),
      spawnedEvent(2, 'se-new', 'claude-code', 'medium'),
    ]);
    const steps = [
      step({ id: 'se-old', step_id: 'implement', updated_at: 1 }),
      step({ id: 'se-new', step_id: 'implement', updated_at: 5 }),
      step({ id: 'se-other', step_id: 'research', updated_at: 2 }),
    ];
    const { result } = renderHook(() => useRunEvents('f1', steps));

    await waitFor(() =>
      expect(result.current.statusByNode.implement).toMatchObject({
        stepExecutionId: 'se-new',
        agentKind: 'claude-code',
        effort: 'medium',
      }),
    );
    expect(result.current.statusByNode.research.agentKind).toBeUndefined();
    expect(result.current.statusByNode.research.effort).toBeUndefined();
  });

  it('isolates feature state and ignores a late history response from the previous feature', async () => {
    let resolveFirst: (events: RunEvent[]) => void = () => undefined;
    listRunEventsSince
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }))
      .mockResolvedValueOnce([
        { ...spawnedEvent(2, 'se-2', 'hermes', 'low'), run_id: 'f2' },
      ]);
    const { result, rerender } = renderHook(
      ({ featureId, executionId }) =>
        useRunEvents(featureId, [step({ id: executionId, feature_id: featureId })]),
      { initialProps: { featureId: 'f1', executionId: 'se-1' } },
    );

    rerender({ featureId: 'f2', executionId: 'se-2' });
    act(() => resolveFirst([spawnedEvent(1, 'se-1', 'opencode', 'max')]));

    await waitFor(() => expect(result.current.events.map((event) => event.offset)).toEqual([2]));
    expect(result.current.statusByNode.research).toMatchObject({
      stepExecutionId: 'se-2',
      agentKind: 'hermes',
      effort: 'low',
    });
  });

  it('keeps live status, retry decisions, and assignments when backfill fails', async () => {
    listRunEventsSince.mockRejectedValue(new Error('history unavailable'));
    const { result } = renderHook(() =>
      useRunEvents('f1', [step({ id: 'se-1', step_id: 'implement', status: 'failed' })]),
    );
    await waitFor(() => expect(listRunEventsSince).toHaveBeenCalledWith('f1', 0));
    expect(result.current.statusByNode.implement.agentKind).toBeUndefined();
    expect(result.current.statusByNode.implement.effort).toBeUndefined();
    await waitFor(() => expect(handlers.run_event?.length).toBeGreaterThan(0));

    act(() => {
      emit('run_event', spawnedEvent(2, 'se-1', 'opencode', 'high'));
      emit('run_event', runEvent({
        offset: 3,
        kind: 'retry_decision',
        payload_json: JSON.stringify({ step_id: 'implement', error_class: 'verdict' }),
      }));
    });

    await waitFor(() => expect(result.current.events).toHaveLength(2));
    expect(result.current.statusByNode.implement).toMatchObject({
      status: 'failed',
      errorClass: 'verdict',
      agentKind: 'opencode',
      effort: 'high',
    });
  });

  it('derives node status/cost/duration from the steps snapshot', () => {
    const steps = [
      step({ id: 'se-a', step_id: 'research', status: 'completed', cost_usd: 0.4, wall_clock_secs: 9 }),
      step({ id: 'se-b', step_id: 'implement', status: 'running' }),
    ];
    const { result } = renderHook(() => useRunEvents('f1', steps));

    expect(result.current.statusByNode.research.status).toBe('completed');
    expect(result.current.statusByNode.research.costUsd).toBe(0.4);
    expect(result.current.statusByNode.research.wallClockSecs).toBe(9);
    expect(result.current.statusByNode.research.stepExecutionId).toBe('se-a');
    expect(result.current.statusByNode.implement.status).toBe('running');
  });

  it('keeps the most recently updated execution per node id (replay/retry)', () => {
    const steps = [
      step({ id: 'se-old', step_id: 'implement', status: 'failed', updated_at: 1 }),
      step({ id: 'se-new', step_id: 'implement', status: 'running', updated_at: 5 }),
    ];
    const { result } = renderHook(() => useRunEvents('f1', steps));
    expect(result.current.statusByNode.implement.status).toBe('running');
    expect(result.current.statusByNode.implement.stepExecutionId).toBe('se-new');
  });

  it('lifts the failure class from a retry_decision run-event', async () => {
    const steps = [step({ id: 'se-b', step_id: 'implement', status: 'failed' })];
    const { result } = renderHook(() => useRunEvents('f1', steps));

    // The listener registers asynchronously (listen() is a Promise).
    await waitFor(() => expect(handlers['run_event']?.length).toBeGreaterThan(0));

    act(() => {
      emit(
        'run_event',
        runEvent({
          kind: 'retry_decision',
          payload_json: JSON.stringify({
            step_id: 'implement',
            error_class: 'agent_failure',
            rule_id: 'agent_failure.fail',
          }),
        }),
      );
    });

    await waitFor(() =>
      expect(result.current.statusByNode.implement.errorClass).toBe('agent_failure'),
    );
    expect(result.current.events).toHaveLength(1);
  });

  it('ignores run-events belonging to a different run', async () => {
    const steps = [step({ id: 'se-b', step_id: 'implement', status: 'failed' })];
    const { result } = renderHook(() => useRunEvents('f1', steps));
    await waitFor(() => expect(handlers['run_event']?.length).toBeGreaterThan(0));

    act(() => {
      emit(
        'run_event',
        runEvent({
          run_id: 'some-other-run',
          kind: 'retry_decision',
          payload_json: JSON.stringify({ step_id: 'implement', error_class: 'verdict' }),
        }),
      );
    });

    // Nothing accumulated: neither the failure class nor the raw feed.
    expect(result.current.statusByNode.implement.errorClass).toBeNull();
    expect(result.current.events).toHaveLength(0);
  });
});
