/**
 * The remote-shadow poll's scheduling contract (UI_REDESIGN_PLAN §4.8). Every
 * assertion here is about *when* `remote_refresh_run` is called, never what it
 * returns: a tick is one tunnel round trip plus a `reload()` that is two more,
 * so a tick nobody can see is pure cost, and a flat retry into a dead tunnel is
 * indistinguishable from a busy runner.
 *
 * Timers are faked and driven by hand — `advanceTimersByTimeAsync` so the
 * awaited IPC inside a tick settles before the next one is scheduled.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Machine, RemoteRunMirror, RunEvent } from '../../types';

const remoteRunForFeature = vi.fn<(featureId: string) => Promise<RemoteRunMirror | null>>();
const remoteRefreshRun =
  vi.fn<(input: { machineId: string; runId: string }) => Promise<RemoteRunMirror | null>>();
const listMachines = vi.fn<() => Promise<Machine[]>>();

vi.mock('../../lib/featureDetail', () => ({
  remoteRunForFeature: (featureId: string) => remoteRunForFeature(featureId),
  remoteRefreshRun: (input: { machineId: string; runId: string }) => remoteRefreshRun(input),
  listMachines: () => listMachines(),
}));

import { useRemoteRun } from './useRemoteRun';

// jsdom's `document.hidden` is a prototype getter with no setter, so a test
// drives it through an own accessor over a local flag.
let hidden = false;
Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });

const mirror = (over: Partial<RemoteRunMirror> = {}): RemoteRunMirror => ({
  machine_id: 'm1',
  run_id: 'r1',
  project_id: 'p1',
  title: 'Add a widget',
  status: 'running',
  error: null,
  feature_id: 'f1',
  pr_url: null,
  pushed_branch: null,
  last_offset: 0,
  created_at: 0,
  updated_at: 1,
  last_notified_status: null,
  ...over,
});

const runEvent = (over: Partial<RunEvent> = {}): RunEvent => ({
  offset: 1,
  run_id: 'r1',
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

async function mount(run: RemoteRunMirror | null) {
  remoteRunForFeature.mockResolvedValue(run);
  const reload = vi.fn();
  const view = renderHook(() =>
    useRemoteRun({ featureId: 'f1', reload, upsertBootstrapPhase: () => {} }),
  );
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  return { reload, ...view };
}

const advance = (ms: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });

beforeEach(() => {
  vi.useFakeTimers();
  hidden = false;
  remoteRunForFeature.mockReset();
  remoteRefreshRun.mockReset();
  remoteRefreshRun.mockResolvedValue(null);
  listMachines.mockReset();
  listMachines.mockResolvedValue([]);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useRemoteRun poll scheduling', () => {
  it('never polls while the document is hidden', async () => {
    hidden = true;
    await mount(mirror());

    await advance(60_000);

    expect(remoteRefreshRun).not.toHaveBeenCalled();
  });

  it('stops polling when the document becomes hidden mid-run', async () => {
    await mount(mirror());
    await advance(3_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(1);

    hidden = true;
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });

    await advance(60_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(1);
  });

  it('catches up immediately when the document becomes visible', async () => {
    hidden = true;
    await mount(mirror());

    hidden = false;
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });

    // No timer was advanced: coming back to the window shows current state
    // rather than waiting out an interval.
    expect(remoteRefreshRun).toHaveBeenCalledTimes(1);
  });

  it('polls no more after unmount, visible or not', async () => {
    const { unmount } = await mount(mirror());
    unmount();

    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await advance(60_000);

    expect(remoteRefreshRun).not.toHaveBeenCalled();
  });

  it('grows the interval across consecutive failures', async () => {
    remoteRefreshRun.mockRejectedValue(new Error('tunnel is gone'));
    await mount(mirror());

    await advance(3_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(1);

    await advance(3_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(1);
    await advance(3_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(2);

    await advance(6_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(2);
    await advance(6_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(3);
  });

  it('caps the backoff', async () => {
    remoteRefreshRun.mockRejectedValue(new Error('tunnel is gone'));
    await mount(mirror());

    await advance(10 * 60_000);
    const callsInTenMinutes = remoteRefreshRun.mock.calls.length;

    // Ten minutes at the ceiling still retries, but nowhere near the 200 ticks
    // a flat 3s would have spent on a tunnel that is not coming back.
    expect(callsInTenMinutes).toBeGreaterThan(10);
    expect(callsInTenMinutes).toBeLessThan(30);
  });

  it('resets the backoff on the first success', async () => {
    remoteRefreshRun.mockRejectedValueOnce(new Error('tunnel hiccup'));
    await mount(mirror());

    await advance(3_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(1);

    await advance(6_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(2);

    await advance(3_000);
    expect(remoteRefreshRun).toHaveBeenCalledTimes(3);
  });

  it('pulls a terminal run exactly once and then holds no timer', async () => {
    const { reload } = await mount(mirror({ status: 'completed' }));

    expect(reload).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);

    await advance(60_000);
    expect(remoteRefreshRun).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('schedules nothing for a locally-run feature', async () => {
    const { reload } = await mount(null);

    expect(vi.getTimerCount()).toBe(0);
    await advance(60_000);
    expect(remoteRefreshRun).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
  });
});

describe('useRemoteRun detached event folding', () => {
  it('continues folding bootstrap phases from the Activity-delivered batch', async () => {
    remoteRunForFeature.mockResolvedValue(mirror());
    const upsertBootstrapPhase = vi.fn();
    const { result } = renderHook(() =>
      useRemoteRun({ featureId: 'f1', reload: () => {}, upsertBootstrapPhase }),
    );
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    act(() => {
      result.current.handleRunEvents([
        runEvent({
          offset: 1,
          kind: 'bootstrap_progress',
          payload_json: JSON.stringify({ phase: 'clone', label: 'Clone', status: 'completed' }),
        }),
        spawnedEvent(2, 'se-1', 'opencode', 'high'),
      ]);
    });

    expect(upsertBootstrapPhase).toHaveBeenCalledWith({
      phase: 'clone',
      label: 'Clone',
      status: 'completed',
    });
    expect(result.current.remoteRunAssignments['se-1']?.agentKind).toBe('opencode');
  });

  it('keeps the greatest valid assignment across reconnect and out-of-order batches', async () => {
    const { result } = await mount(mirror());

    act(() => {
      result.current.handleRunEvents([
        spawnedEvent(8, 'se-1', 'claude-code', null),
        spawnedEvent(4, 'se-1', 'opencode', 'high'),
        runEvent({
          offset: 9,
          kind: 'agent_spawned',
          payload_json: JSON.stringify({
            step_execution_id: 'se-2',
            agent_kind: 'hermes',
            effort: 'unsupported',
          }),
        }),
      ]);
      // Offset 4 is the reconnect re-delivering a row already folded; offset
      // 10 is genuinely new. A re-delivered offset is refused by identity, so
      // the log's own immutability — not the payload — settles the duplicate.
      result.current.handleRunEvents([
        spawnedEvent(4, 'se-1', 'duplicate', 'low'),
        spawnedEvent(10, 'se-2', 'hermes', 'medium'),
      ]);
    });

    expect(result.current.remoteRunAssignments).toEqual({
      'se-1': {
        stepExecutionId: 'se-1',
        agentKind: 'claude-code',
        effort: null,
        offset: 8,
      },
      'se-2': {
        stepExecutionId: 'se-2',
        agentKind: 'hermes',
        effort: 'medium',
        offset: 10,
      },
    });
    // Ordered by durable offset, not by the order the polls delivered them —
    // the same fold the local feed runs, so the strip reads alike either way.
    expect(result.current.remoteRunEvents.map((event) => event.offset)).toEqual([4, 8, 9, 10]);

    const stableAssignments = result.current.remoteRunAssignments;
    act(() => result.current.handleRunEvents([spawnedEvent(10, 'se-2', 'retry', 'low')]));
    expect(result.current.remoteRunAssignments).toBe(stableAssignments);
  });

  it('retains an assignment after its spawn row is evicted from the activity window', async () => {
    const { result } = await mount(mirror());
    const laterEvents = Array.from({ length: 501 }, (_, index) =>
      runEvent({ offset: index + 2 }),
    );

    act(() => {
      result.current.handleRunEvents([
        spawnedEvent(1, 'se-1', 'hermes', null),
        ...laterEvents,
      ]);
    });

    expect(result.current.remoteRunEvents).toHaveLength(500);
    expect(result.current.remoteRunEvents[0]?.offset).toBe(3);
    expect(result.current.remoteRunAssignments['se-1']).toMatchObject({
      agentKind: 'hermes',
      effort: null,
      offset: 1,
    });
  });

  it('isolates accumulated events and assignments when the remote run changes', async () => {
    const reload = vi.fn();
    remoteRunForFeature.mockImplementation(async (featureId) =>
      featureId === 'f1'
        ? mirror()
        : mirror({ feature_id: featureId, run_id: 'r2' }),
    );
    const { result, rerender } = renderHook(
      ({ featureId }) =>
        useRemoteRun({ featureId, reload, upsertBootstrapPhase: () => {} }),
      { initialProps: { featureId: 'f1' } },
    );
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    act(() => result.current.handleRunEvents([spawnedEvent(1, 'se-1', 'opencode', 'max')]));

    rerender({ featureId: 'f2' });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(result.current.remoteRunEvents).toEqual([]);
    expect(result.current.remoteRunAssignments).toEqual({});
  });
});
