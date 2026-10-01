import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { LOCAL_MACHINE } from '../lib/newDiscovery';
import { getRunnerCompatibility } from '../lib/runner';
import { invalidateRunnerCompatibility } from '../lib/runnerCompatibility';
import type { RunnerCompatibilityReport } from '../types';
import { useRunnerCompatibility } from './useRunnerCompatibility';

vi.mock('../lib/runner', () => ({ getRunnerCompatibility: vi.fn() }));

const behind: RunnerCompatibilityReport = {
  verdict: 'runner_behind',
  runner: '1.1.0',
  runner_channel: 'stable',
  app: '1.2.0',
  app_channel: 'stable',
  message: 'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable) — upgrade the runner.',
};

const compatible: RunnerCompatibilityReport = {
  verdict: 'compatible',
  version: '1.2.0',
  channel: 'stable',
  message: 'demeteo-runner 1.2.0 (stable) on other matches Demeteo.',
};

beforeEach(() => {
  invalidateRunnerCompatibility();
  vi.mocked(getRunnerCompatibility).mockReset();
});

describe('useRunnerCompatibility', () => {
  it.each([
    ['run here', ''],
    ['the desktop host', LOCAL_MACHINE],
  ])('makes no IPC call for %s', (_, machineId) => {
    const { result } = renderHook(() => useRunnerCompatibility(machineId));

    expect(result.current).toMatchObject({ report: null, loading: false });
    act(() => result.current.refresh());
    expect(getRunnerCompatibility).not.toHaveBeenCalled();
  });

  it('probes a remote machine once and returns its verdict', async () => {
    vi.mocked(getRunnerCompatibility).mockResolvedValue(behind);

    const { result } = renderHook(() => useRunnerCompatibility('box'));

    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.report).toEqual(behind));
    expect(result.current.loading).toBe(false);
    expect(getRunnerCompatibility).toHaveBeenCalledTimes(1);
    expect(getRunnerCompatibility).toHaveBeenCalledWith('box');
  });

  it('leaves the report null when the probe fails', async () => {
    vi.mocked(getRunnerCompatibility).mockRejectedValue(new Error('ssh: connection refused'));

    const { result } = renderHook(() => useRunnerCompatibility('box'));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.report).toBeNull();
  });

  it('drops a slow answer for a machine that is no longer selected', async () => {
    let answerBox: (report: RunnerCompatibilityReport) => void = () => {};
    vi.mocked(getRunnerCompatibility).mockImplementation((machineId) =>
      machineId === 'box'
        ? new Promise((resolve) => {
            answerBox = resolve;
          })
        : Promise.resolve(compatible),
    );

    const { result, rerender } = renderHook(({ id }) => useRunnerCompatibility(id), {
      initialProps: { id: 'box' },
    });
    rerender({ id: 'other' });
    await waitFor(() => expect(result.current.report).toEqual(compatible));

    await act(async () => answerBox(behind));
    expect(result.current).toMatchObject({ report: compatible, loading: false });
  });

  it('clears the previous verdict when switching to the local machine', async () => {
    vi.mocked(getRunnerCompatibility).mockResolvedValue(behind);
    const { result, rerender } = renderHook(({ id }) => useRunnerCompatibility(id), {
      initialProps: { id: 'box' },
    });
    await waitFor(() => expect(result.current.report).toEqual(behind));

    rerender({ id: LOCAL_MACHINE });
    expect(result.current).toMatchObject({ report: null, loading: false });
  });

  it('refresh re-probes past the cache', async () => {
    vi.mocked(getRunnerCompatibility).mockResolvedValueOnce(behind).mockResolvedValueOnce(compatible);
    const { result } = renderHook(() => useRunnerCompatibility('box'));
    await waitFor(() => expect(result.current.report).toEqual(behind));

    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.report).toEqual(compatible));
    expect(getRunnerCompatibility).toHaveBeenCalledTimes(2);
  });
});
