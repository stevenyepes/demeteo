// What this file pins down, and why each claim needs a test rather than a
// comment:
//
// `step_progress` is not a transition feed. `turn.rs` emits one per streamed
// text delta, and the backend's `PROGRESS_THROTTLE_MS` gates only the
// *persisted* run-event row — the UI receives every single one. So the fetch
// count under a burst is the property to assert; a reader cannot tell from
// `useFeatureRun.ts` alone how often that handler fires.
//
// The interim patch and the pipeline total are two halves of one bug. The
// header's cost must stay live between reloads (the patch) while never
// becoming a single step's spend (audit F19). A test that only checked "the
// number moved" would pass on the bug it replaced.
//
// The backend double answers exactly what one reload asks for and rejects
// anything else (AGENTS §7): a command that silently starts mattering surfaces
// here as a failure rather than as an `undefined` the hook renders around.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { StepExecution } from '../../types';
import type { HarnessOverrides } from './useHarnessOverrides';
import { useFeatureRun } from './useFeatureRun';

const FEATURE_ID = 'f-1';
const PROJECT_ID = 'p-1';

interface StepProgressPayload {
  feature_id: string;
  step_id: string;
  status: string;
  cost_usd: number | null;
  tokens: number | null;
  wall_clock_secs: number | null;
  cache_read_input_tokens: number | null;
  cache_creation_input_tokens: number | null;
}

function stepRow(overrides: Partial<StepExecution> & { id: string; step_id: string }): StepExecution {
  return {
    feature_id: FEATURE_ID,
    step_index: 0,
    step_kind: 'agent',
    status: 'running',
    cost_usd: 0,
    tokens: 0,
    wall_clock_secs: 0,
    artifact_paths: [],
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

function progress(overrides: Partial<StepProgressPayload> & { step_id: string }): StepProgressPayload {
  return {
    feature_id: FEATURE_ID,
    status: 'running',
    cost_usd: null,
    tokens: null,
    wall_clock_secs: null,
    cache_read_input_tokens: null,
    cache_creation_input_tokens: null,
    ...overrides,
  };
}

/** Only `adoptFeatureModel` / `probeForFeature` are reachable from the hook;
 *  the rest of the shape is spelled out so a field added to `HarnessOverrides`
 *  breaks here instead of being silently absent. */
function fakeOverrides(): HarnessOverrides {
  return {
    machineAgents: [],
    availableModels: [],
    selectedModel: '',
    setSelectedModel: () => {},
    isLoadingModels: false,
    availableAgents: [],
    selectedAgent: '',
    selectedEffort: '',
    setSelectedEffort: () => {},
    featureAgentKind: 'opencode',
    retryEffortLevels: [],
    onAgentChange: () => {},
    adoptFeatureModel: vi.fn(),
    probeForFeature: vi.fn(),
  };
}

interface Backend {
  steps: StepExecution[];
  status: string;
  stepListCalls: number;
  featureGetCalls: number;
  /** When set, `step_list_for_run` resolves only once this settles — the
   *  in-flight fetch a second `reload()` has to queue behind. */
  gate: Promise<void> | null;
}

let backend: Backend;
const eventHandlers = new Map<string, (event: { payload: unknown }) => void>();

/** Fresh row objects per call, exactly as IPC hands them over: the identity
 *  test is meaningless against a double that returns the same objects. */
function wireWithFreshRows(rows: StepExecution[]): StepExecution[] {
  return rows.map((row) => ({ ...row, artifact_paths: [...row.artifact_paths] }));
}

beforeEach(() => {
  backend = {
    steps: [stepRow({ id: 'se-1', step_id: 'implement' })],
    status: 'running',
    stepListCalls: 0,
    featureGetCalls: 0,
    gate: null,
  };
  eventHandlers.clear();

  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    switch (cmd) {
      case 'step_list_for_run': {
        backend.stepListCalls += 1;
        const snapshot = wireWithFreshRows(backend.steps);
        return backend.gate ? backend.gate.then(() => snapshot) : Promise.resolve(snapshot);
      }
      case 'feature_get':
        backend.featureGetCalls += 1;
        return Promise.resolve({
          id: FEATURE_ID,
          project_id: PROJECT_ID,
          status: backend.status,
          title: 'Coalesced telemetry',
          description: '',
          agent_kind: 'opencode',
          model: 'claude-opus-4',
        });
      default:
        return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
    }
  }) as unknown as typeof invoke);

  vi.mocked(listen).mockImplementation(((event: string, handler: (e: { payload: unknown }) => void) => {
    eventHandlers.set(event, handler);
    return Promise.resolve(() => eventHandlers.delete(event));
  }) as unknown as typeof listen);
});

afterEach(() => {
  vi.useRealTimers();
});

function emitProgress(payload: StepProgressPayload): void {
  const handler = eventHandlers.get('step_progress');
  if (!handler) throw new Error('nothing subscribed to step_progress');
  handler({ payload });
}

function mountRun() {
  return renderHook(() =>
    useFeatureRun({
      featureId: FEATURE_ID,
      projectId: PROJECT_ID,
      initialTitle: 'Coalesced telemetry',
      overrides: fakeOverrides(),
    }),
  );
}

/** Let the mount reload land and the event subscriptions resolve. */
async function settle(): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}

describe('useFeatureRun under a step_progress burst', () => {
  it('coalesces a burst into a handful of fetches', async () => {
    vi.useFakeTimers();
    const { result } = mountRun();
    await settle();

    expect(backend.stepListCalls).toBe(1);

    for (let i = 0; i < 10; i += 1) {
      act(() => emitProgress(progress({ step_id: 'implement', cost_usd: 0.01 * (i + 1) })));
    }
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });

    for (let i = 0; i < 10; i += 1) {
      act(() => emitProgress(progress({ step_id: 'implement', cost_usd: 0.2 + 0.01 * (i + 1) })));
    }
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });

    expect(backend.stepListCalls).toBe(3);
    expect(backend.featureGetCalls).toBe(3);
    expect(result.current.steps).toHaveLength(1);
  });

  it('keeps the streaming step live between reloads without a fetch', async () => {
    vi.useFakeTimers();
    backend.steps = [
      stepRow({ id: 'se-1', step_id: 'implement', cost_usd: 0.1, tokens: 100, wall_clock_secs: 10 }),
    ];
    const { result } = mountRun();
    await settle();

    const fetchesBefore = backend.stepListCalls;
    act(() =>
      emitProgress(
        progress({
          step_id: 'implement',
          cost_usd: 0.25,
          tokens: 400,
          wall_clock_secs: 30,
          cache_read_input_tokens: 900,
          cache_creation_input_tokens: 120,
        }),
      ),
    );

    expect(backend.stepListCalls).toBe(fetchesBefore);
    expect(result.current.steps[0].cost_usd).toBeCloseTo(0.25);
    expect(result.current.steps[0].tokens).toBe(400);
    expect(result.current.steps[0].wall_clock_secs).toBe(30);
    expect(result.current.totalCost).toBeCloseTo(0.25);
    expect(result.current.tokens).toBe(400);
    expect(result.current.duration).toBe('30s');
    expect(result.current.cacheReadTokens).toBe(900);
    expect(result.current.cacheCreationTokens).toBe(120);
  });

  it('never sets the pipeline total to one step\'s cost', async () => {
    vi.useFakeTimers();
    backend.steps = [
      stepRow({ id: 'se-1', step_id: 'spec', status: 'completed', cost_usd: 0.1, tokens: 100 }),
      stepRow({ id: 'se-2', step_id: 'implement', step_index: 1, cost_usd: 0.4, tokens: 400 }),
    ];
    const { result } = mountRun();
    await settle();

    expect(result.current.totalCost).toBeCloseTo(0.5);

    act(() => emitProgress(progress({ step_id: 'implement', cost_usd: 0.45, tokens: 450 })));

    expect(result.current.totalCost).toBeCloseTo(0.55);
    expect(result.current.tokens).toBe(550);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });

    expect(result.current.totalCost).toBeCloseTo(0.5);
  });

  it('patches the most recent execution row for the event\'s step', async () => {
    vi.useFakeTimers();
    backend.steps = [
      stepRow({ id: 'se-1', step_id: 'implement', status: 'failed', cost_usd: 0.1, updated_at: 10 }),
      stepRow({ id: 'se-2', step_id: 'implement', step_index: 1, cost_usd: 0.2, updated_at: 20 }),
    ];
    const { result } = mountRun();
    await settle();

    act(() => emitProgress(progress({ step_id: 'implement', cost_usd: 0.9 })));

    expect(result.current.steps[0].cost_usd).toBeCloseTo(0.1);
    expect(result.current.steps[1].cost_usd).toBeCloseTo(0.9);
  });

  it('ignores a burst aimed at another feature', async () => {
    vi.useFakeTimers();
    const { result } = mountRun();
    await settle();

    act(() =>
      emitProgress(progress({ feature_id: 'f-other', step_id: 'implement', cost_usd: 9 })),
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });

    expect(backend.stepListCalls).toBe(1);
    expect(result.current.totalCost).toBe(0);
  });
});

describe('useFeatureRun step identity', () => {
  it('leaves the steps array untouched when a reload changes nothing', async () => {
    vi.useFakeTimers();
    const { result } = mountRun();
    await settle();

    const before = result.current.steps;
    const firstRow = before[0];

    await act(async () => {
      await result.current.reload();
    });

    expect(backend.stepListCalls).toBe(2);
    expect(result.current.steps).toBe(before);
    expect(result.current.steps[0]).toBe(firstRow);
  });

  it('gives only the changed row a new identity', async () => {
    vi.useFakeTimers();
    backend.steps = [
      stepRow({ id: 'se-1', step_id: 'spec', status: 'completed' }),
      stepRow({ id: 'se-2', step_id: 'implement', step_index: 1 }),
    ];
    const { result } = mountRun();
    await settle();

    const before = result.current.steps;
    backend.steps = [
      backend.steps[0],
      { ...backend.steps[1], status: 'completed', updated_at: 99 },
    ];

    await act(async () => {
      await result.current.reload();
    });

    expect(result.current.steps).not.toBe(before);
    expect(result.current.steps[0]).toBe(before[0]);
    expect(result.current.steps[1].status).toBe('completed');
  });
});

describe('useFeatureRun reload contract', () => {
  it('settles only once the awaited data is in state', async () => {
    vi.useFakeTimers();
    const { result } = mountRun();
    await settle();

    backend.steps = [stepRow({ id: 'se-1', step_id: 'implement', status: 'completed' })];

    await act(async () => {
      await result.current.reload();
    });

    expect(result.current.steps[0].status).toBe('completed');
  });

  // `useRemoteRun`, `useRerunActions` and `useFeatureMr` all call `reload()`
  // without awaiting it, right after a mutation the user is watching for. A
  // caller's reload therefore has to jump whatever the progress feed left
  // queued, or a decided gate keeps rendering as pending for the floor's
  // length.
  it('starts a caller-driven reload without waiting out the progress floor', async () => {
    vi.useFakeTimers();
    const { result } = mountRun();
    await settle();

    act(() => emitProgress(progress({ step_id: 'implement', cost_usd: 0.01 })));
    const beforeReload = backend.stepListCalls;

    act(() => {
      void result.current.reload();
    });

    expect(backend.stepListCalls).toBe(beforeReload + 1);
    await settle();
  });

  it('queues behind an in-flight fetch instead of joining it', async () => {
    vi.useFakeTimers();
    const { result } = mountRun();
    await settle();

    let openGate = () => {};
    backend.gate = new Promise<void>((resolve) => {
      openGate = resolve;
    });

    let firstReload: Promise<void> = Promise.resolve();
    await act(async () => {
      firstReload = result.current.reload();
      await Promise.resolve();
    });

    // Written after the in-flight fetch took its snapshot: a `reload` that
    // resolved off that fetch would report the pre-write state as current.
    backend.gate = null;
    backend.steps = [stepRow({ id: 'se-1', step_id: 'implement', status: 'completed' })];

    let secondReload: Promise<void> = Promise.resolve();
    act(() => {
      secondReload = result.current.reload();
    });

    await act(async () => {
      openGate();
      await Promise.all([firstReload, secondReload]);
    });

    expect(backend.stepListCalls).toBe(3);
    expect(result.current.steps[0].status).toBe('completed');
  });

  // The manual sync writes a real `step_executions` row so the resolution can
  // stream to an id the inspector subscribes to — and `step_executions` is also
  // this hook's rollup input, with nothing on the row marking it out-of-band.
  // A resolution the user tried once and abandoned therefore reported a run
  // that had already finished as failed, permanently.
  it('does not let an out-of-band sync restate a finished run', async () => {
    vi.useFakeTimers();
    backend.status = 'completed';
    backend.steps = [
      stepRow({ id: 'se-1', step_id: 'implement', status: 'completed' }),
      stepRow({
        id: 'se-1-s-sync-manual',
        step_id: 's-sync-manual',
        step_kind: 'sync',
        status: 'failed',
        cost_usd: 2.5,
      }),
    ];

    const { result } = mountRun();
    await settle();

    expect(result.current.status).toBe('completed');
    // The dollars are the feature's either way, so the spend still counts it.
    expect(result.current.totalCost).toBe(2.5);
  });
});
