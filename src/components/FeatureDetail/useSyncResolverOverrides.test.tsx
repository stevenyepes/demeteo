/**
 * The banner's picker probed models for the harness the *run* was launched
 * with, which is the wrong tier: the resolver chain reads the project's
 * conflict-resolver setting first. It also probed before that harness was
 * known, and `probeForFeature` latches on its first non-empty answer — so the
 * placeholder it started with was the one the view kept.
 *
 * These pin both halves: nothing is probed until the backend has answered, and
 * what is probed is the backend's answer.
 */
import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { SyncResolverView } from '../../types';

const getSyncResolver = vi.fn<(featureId: string) => Promise<SyncResolverView>>();
const probeForFeature =
  vi.fn<(input: { agentKind: string | null | undefined; projectId: string }) => void>();

vi.mock('../../lib/featureSync', () => ({
  getSyncResolver: (featureId: string) => getSyncResolver(featureId),
}));

vi.mock('./useHarnessOverrides', () => ({
  useHarnessOverrides: () => ({ probeForFeature, featureAgentKind: 'opencode' }),
}));

import { useSyncResolverOverrides } from './useSyncResolverOverrides';

function mount(conflicted: boolean) {
  return renderHook(() =>
    useSyncResolverOverrides({ featureId: 'f-1', projectId: 'p-1', conflicted }),
  );
}

describe('useSyncResolverOverrides', () => {
  it('probes the harness the backend says would run, not the run\'s own', async () => {
    getSyncResolver.mockResolvedValue({ agent_kind: 'codex', model: 'gpt-5-codex', effort: 'low' });
    const { result } = mount(true);

    await waitFor(() => expect(probeForFeature).toHaveBeenCalled());
    expect(probeForFeature).toHaveBeenCalledWith({ agentKind: 'codex', projectId: 'p-1' });
    expect(result.current.inherited?.agent_kind).toBe('codex');
  });

  it('probes nothing before the answer lands', async () => {
    let answer: (r: SyncResolverView) => void = () => {};
    getSyncResolver.mockReturnValue(new Promise<SyncResolverView>((res) => { answer = res; }));
    mount(true);

    await waitFor(() => expect(getSyncResolver).toHaveBeenCalled());
    expect(probeForFeature).not.toHaveBeenCalled();

    answer({ agent_kind: 'hermes', model: null, effort: 'high' });
    await waitFor(() => expect(probeForFeature).toHaveBeenCalledWith({
      agentKind: 'hermes',
      projectId: 'p-1',
    }));
  });

  it('asks nothing at all until there is a conflict on screen', async () => {
    mount(false);

    await waitFor(() => expect(getSyncResolver).not.toHaveBeenCalled());
    expect(probeForFeature).not.toHaveBeenCalled();
  });
});
