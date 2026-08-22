/**
 * The two things this hook has to get right are both about an answer arriving
 * after the question stopped being worth asking: a count that lands during the
 * sync it was taken before, and a rejection that lands as silence.
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { FeatureDrift } from '../../types';

const getFeatureDrift = vi.fn<(featureId: string, refresh?: boolean) => Promise<FeatureDrift>>();

vi.mock('../../lib/featureSync', () => ({
  getFeatureDrift: (featureId: string, refresh?: boolean) => getFeatureDrift(featureId, refresh),
}));

import { useFeatureDrift } from './useFeatureDrift';

function answer(behind: number | null): FeatureDrift {
  return {
    divergence: { behind, ahead: 0 },
    base_ref: 'origin/main',
    fetched: false,
    checked_at: 1,
  };
}

describe('useFeatureDrift', () => {
  it('reads without a fetch until somebody asks for one', async () => {
    getFeatureDrift.mockResolvedValue(answer(3));
    const { result } = renderHook(() => useFeatureDrift({ featureId: 'f-1', enabled: true }));

    await waitFor(() => expect(result.current.drift?.divergence.behind).toBe(3));
    expect(getFeatureDrift).toHaveBeenCalledWith('f-1', false);

    await act(async () => {
      result.current.refresh();
    });
    expect(getFeatureDrift).toHaveBeenLastCalledWith('f-1', true);
  });

  it('keeps a count taken before a sync from landing during it', async () => {
    let land: (d: FeatureDrift) => void = () => {};
    getFeatureDrift.mockReturnValue(new Promise<FeatureDrift>((res) => { land = res; }));
    const { result, rerender } = renderHook(
      ({ enabled }) => useFeatureDrift({ featureId: 'f-1', enabled }),
      { initialProps: { enabled: true } },
    );

    rerender({ enabled: false });
    await act(async () => {
      land(answer(3));
    });

    expect(result.current.drift).toBeNull();
  });

  it('renders a failed read as unmeasured rather than as nothing to pull', async () => {
    getFeatureDrift.mockRejectedValue('the repository is not on this machine');
    const { result } = renderHook(() => useFeatureDrift({ featureId: 'f-1', enabled: true }));

    await waitFor(() => expect(result.current.drift).not.toBeNull());
    expect(result.current.drift?.divergence.behind).toBeNull();
    expect(result.current.refreshing).toBe(false);
  });
});
