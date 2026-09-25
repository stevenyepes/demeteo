/**
 * What a rewind is allowed to change, and what it is not.
 *
 * Retry and Replay used to carry the inspector's harness/model/effort trio into
 * `step_retry` / `replay_from_step`, which write the *feature-wide* tier: one
 * "run this failed node on the other harness" silently re-pointed every step
 * after it too. The trio now belongs to the per-node Assignment control, so the
 * claim under test is a negative one, and negatives rot quietly — nothing else
 * in the suite notices a rewind that started carrying a value again.
 */
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { RemoteRunMirror } from '../../types';

const retryStep = vi.fn<(input: unknown) => Promise<void>>();
const replayFromStep = vi.fn<(input: unknown) => Promise<void>>();
const remoteRetryStep = vi.fn<(input: unknown) => Promise<void>>();
const remoteReplayStep = vi.fn<(input: unknown) => Promise<void>>();

vi.mock('../../lib/features', () => ({
  retryStep: (input: unknown) => retryStep(input),
  replayFromStep: (input: unknown) => replayFromStep(input),
  remoteRetryStep: (input: unknown) => remoteRetryStep(input),
  remoteReplayStep: (input: unknown) => remoteReplayStep(input),
  isBlockingError: () => false,
}));

vi.mock('../../lib/featureDetail', () => ({
  cancelFeature: vi.fn().mockResolvedValue(undefined),
  remoteCancelRun: vi.fn().mockResolvedValue(undefined),
}));

import { useRerunActions } from './useRerunActions';

const REMOTE_RUN = { machine_id: 'm-7', run_id: 'r-42' } as RemoteRunMirror;

function mount(remoteRun: RemoteRunMirror | null) {
  return renderHook(() =>
    useRerunActions({
      featureId: 'f-1',
      remoteRun,
      refreshRemoteRun: vi.fn(),
      reload: vi.fn(),
      setFeatureStatus: vi.fn(),
    }),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  retryStep.mockResolvedValue(undefined);
  replayFromStep.mockResolvedValue(undefined);
  remoteRetryStep.mockResolvedValue(undefined);
  remoteReplayStep.mockResolvedValue(undefined);
});

describe('useRerunActions', () => {
  it('retries a local step without re-pinning the feature', async () => {
    const { result } = mount(null);

    await act(() => result.current.handleRetryStep('se-1'));

    expect(retryStep).toHaveBeenCalledWith({
      stepExecutionId: 'se-1',
      newModel: null,
      newAgent: null,
      newEffort: null,
    });
  });

  it('retries a detached step without re-pinning the feature', async () => {
    const { result } = mount(REMOTE_RUN);

    await act(() => result.current.handleRetryStep('se-1'));

    expect(remoteRetryStep).toHaveBeenCalledWith({
      machineId: 'm-7',
      runId: 'r-42',
      stepExecutionId: 'se-1',
      model: null,
      agentKind: null,
      effort: null,
    });
  });

  it('replays a local step without re-pinning the feature', async () => {
    const { result } = mount(null);

    act(() => result.current.startReplay({ id: 'se-2', name: 'implement', downstreamCount: 3 }, null));
    await act(() => result.current.handleReplayFromStep());

    expect(replayFromStep).toHaveBeenCalledWith({
      stepExecutionId: 'se-2',
      newModel: null,
      newAgent: null,
      newEffort: null,
    });
  });

  it('replays a detached step without re-pinning the feature', async () => {
    const { result } = mount(REMOTE_RUN);

    act(() => result.current.startReplay({ id: 'se-2', name: 'implement', downstreamCount: 3 }, null));
    await act(() => result.current.handleReplayFromStep());

    expect(remoteReplayStep).toHaveBeenCalledWith({
      machineId: 'm-7',
      runId: 'r-42',
      stepExecutionId: 'se-2',
      model: null,
      agentKind: null,
      effort: null,
    });
  });
});
