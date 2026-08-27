/**
 * The other half of the sync policy.
 *
 * `lib/syncPanel.ts` decides which affordances exist and had the only tests;
 * this is the single place that knows what each of them reaches for, and an
 * intent wired to the wrong call passes tsc, biome and the whole suite. The two
 * halves were split so they could not disagree, which only holds while both are
 * pinned.
 *
 * The `review` pair is the one worth spelling out: `merge_commit_sha^` names the
 * pre-merge tip only while the resolution is a single merge commit, so a
 * resolver that added a follow-up commit — which agents do routinely — makes the
 * first parent that follow-up's parent and the review silently omits the merge
 * itself.
 */
import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { SyncSessionView } from '../../types';
import type { HarnessOverrides } from './useHarnessOverrides';
import type { SyncResolverSelection } from './useSyncResolverOverrides';
import { useSyncActions } from './useSyncActions';
import type { SyncSession } from './useSyncSession';

const session = (over: Partial<SyncSessionView> = {}): SyncSessionView => ({
  feature_id: 'f-1',
  machine_id: 'local',
  repo_dir: '/repos/demeteo',
  feature_branch: 'feature/f-1',
  base_branch: 'origin/master',
  status: 'conflicted',
  worktree_path: '/repos/demeteo_wt_sync_feature-f-1',
  head_before: 'aaaaaaa1111',
  merge_commit_sha: null,
  conflict_files: [
    { path: 'src/lib.rs', kind: 'both-modified' },
    { path: 'src/main.rs', kind: 'added-by-them' },
  ],
  raw_error: null,
  blocked_stage: null,
  pushed_at: null,
  user_may_intervene: true,
  attempts: 1,
  created_at: 0,
  updated_at: 0,
  ...over,
});

function harness(over: Partial<SyncResolverSelection['overrides']> = {}): SyncResolverSelection {
  return {
    inherited: null,
    overrides: {
      machineAgents: [],
      availableModels: [],
      selectedModel: '',
      setSelectedModel: () => {},
      isLoadingModels: false,
      availableAgents: ['opencode'],
      selectedAgent: '',
      selectedEffort: '',
      setSelectedEffort: () => {},
      featureAgentKind: 'opencode',
      retryEffortLevels: [],
      onAgentChange: () => {},
      adoptFeatureModel: () => {},
      probeForFeature: () => {},
      ...over,
    } satisfies HarnessOverrides,
  };
}

function mount(over: { session?: SyncSessionView | null; resolver?: SyncResolverSelection } = {}) {
  const calls = {
    startSync: vi.fn(async () => {}),
    resolve: vi.fn(async () => {}),
    abort: vi.fn(async () => {}),
    publish: vi.fn(async () => {}),
    discard: vi.fn(async () => {}),
    refresh: vi.fn(async () => {}),
    reconcile: vi.fn(async () => {}),
  };
  const sync: SyncSession = {
    session: over.session === undefined ? session() : over.session,
    divergence: null,
    pending: null,
    ...calls,
  };
  const refreshDrift = vi.fn();
  const openDiffRange = vi.fn();
  const showResolverStream = vi.fn();
  const { result } = renderHook(() =>
    useSyncActions({
      sync,
      resolver: over.resolver ?? harness(),
      refreshDrift,
      openDiffRange,
      showResolverStream,
    }),
  );
  return { act: result.current, calls, refreshDrift, openDiffRange, showResolverStream };
}

describe('useSyncActions', () => {
  it.each([
    ['sync', 'startSync'],
    ['abort', 'abort'],
    ['publish', 'publish'],
    ['discard', 'discard'],
  ] as const)('sends %s to the call that performs it', (intent, call) => {
    const { act, calls } = mount();

    act(intent);

    expect(calls[call]).toHaveBeenCalledTimes(1);
    for (const [name, spy] of Object.entries(calls)) {
      if (name !== call) expect(spy).not.toHaveBeenCalled();
    }
  });

  /** The resolver runs against the paths the *row* names, not a selection held
   *  beside it, and the harness is whatever the picker resolved to. */
  it('resolves the conflicted paths the session names, under the chosen harness', () => {
    const { act, calls } = mount({
      resolver: harness({ selectedAgent: 'claude-code', selectedModel: 'sonnet', selectedEffort: 'high' }),
    });

    act('resolve');

    expect(calls.resolve).toHaveBeenCalledWith(['src/lib.rs', 'src/main.rs'], {
      agentKind: 'claude-code',
      model: 'sonnet',
      effort: 'high',
    });
  });

  /** An untouched picker means "inherit", which the backend answers by walking
   *  its own resolver chain — an empty string would pin a harness named ''. */
  it('sends nulls for a picker nobody touched', () => {
    const { act, calls } = mount();

    act('resolve');

    expect(calls.resolve).toHaveBeenCalledWith(expect.anything(), {
      agentKind: null,
      model: null,
      effort: null,
    });
  });

  it('reviews the merge alone, from the recorded pre-merge tip', () => {
    const { act, openDiffRange } = mount({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222' }),
    });

    act('review');

    expect(openDiffRange).toHaveBeenCalledWith({
      baseRef: 'aaaaaaa1111',
      headRef: 'c0ffeec2222',
    });
  });

  /** The pre-merge tip is unrecoverable once the merge lands, so a session
   *  without one has no honest base — and a diff against a guess is worse than
   *  no diff. `describeSyncPanel` offers no Review row here; this is the second
   *  half of that refusal. */
  it('opens no diff when the session recorded no pre-merge tip', () => {
    const { act, openDiffRange } = mount({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', head_before: null }),
    });

    act('review');

    expect(openDiffRange).not.toHaveBeenCalled();
  });

  /** Both halves: the row is re-read *and* the count is re-fetched. Dropping
   *  either leaves the press paying for one and showing the other. */
  it('re-reads the row and re-fetches the count on a refresh', () => {
    const { act, calls, refreshDrift } = mount();

    act('refresh');

    expect(calls.refresh).toHaveBeenCalledTimes(1);
    expect(refreshDrift).toHaveBeenCalledTimes(1);
  });

  /** Two presses, one call, and which of them it was is the argument: the merge
   *  and the reset are different git operations behind one reconcile, and a
   *  shared intent would send the reset's move for the merge's row. */
  it.each(['reconcile', 'reset_onto_origin'] as const)(
    'sends %s to the reconcile as the move it names',
    (intent) => {
      const { act, calls } = mount();

      act(intent);

      expect(calls.reconcile).toHaveBeenCalledWith(intent);
      expect(calls.startSync).not.toHaveBeenCalled();
    },
  );

  it('sends the watch to the resolver stream and to no backend call', () => {
    const { act, calls, showResolverStream } = mount();

    act('watch');

    expect(showResolverStream).toHaveBeenCalledTimes(1);
    for (const spy of Object.values(calls)) expect(spy).not.toHaveBeenCalled();
  });
});
