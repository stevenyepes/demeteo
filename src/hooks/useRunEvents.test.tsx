/**
 * `useRunEvents` (P2.2): the single overlay derivation both run-mode surfaces
 * share. These prove the two inputs fold into one `statusByNode` shape — the
 * authoritative `step_executions` snapshot for status/cost/duration, and the
 * `run_events` stream for the failure class a step row can't carry — and that
 * events for other runs are ignored.
 */
import { renderHook, act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

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

afterEach(() => {
  for (const k of Object.keys(handlers)) delete handlers[k];
});

describe('useRunEvents', () => {
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
