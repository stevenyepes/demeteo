/**
 * What the one Sync pane renders, driven by the same `describeSyncPanel` the
 * app uses — a hand-built model would assert the fixture rather than the
 * mapping, and the mapping is the part five separate banners used to disagree
 * about.
 *
 * Colour is asserted through `TONE_CHIP`/`data-tone`, never a literal class: a
 * pane that spells `bg-amber-500/10` itself passes a literal assertion while
 * re-opening audit F27.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { describeSyncPanel } from '../../../lib/syncPanel';
import type { FeatureDrift, SyncSessionView } from '../../../types';
import type { HarnessOverrides } from '../useHarnessOverrides';
import type { SyncResolverSelection } from '../useSyncResolverOverrides';
import { SyncPanel } from './SyncPanel';

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
  conflict_files: [{ path: 'src/lib.rs', kind: 'both-modified' }],
  raw_error: 'CONFLICT (content): Merge conflict in src/lib.rs',
  pushed_at: null,
  user_may_intervene: true,
  attempts: 1,
  created_at: 0,
  updated_at: 0,
  ...over,
});

const behindDrift: FeatureDrift = {
  divergence: { behind: 3, ahead: 1 },
  base_ref: 'origin/master',
  fetched: true,
  checked_at: 0,
};

const resolverSelection: SyncResolverSelection = {
  inherited: { agent_kind: 'opencode', model: null, effort: 'medium' },
  overrides: {
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
  } satisfies HarnessOverrides,
};

function mount(
  over: {
    session?: SyncSessionView | null;
    drift?: FeatureDrift | null;
    onAction?: (intent: string) => void;
  } = {},
) {
  const sessionRow = over.session === undefined ? session() : over.session;
  const drift = over.drift ?? null;
  const model = describeSyncPanel({ session: sessionRow, drift, canSync: true });
  const result = render(
    <SyncPanel
      model={model}
      session={sessionRow}
      drift={drift}
      resolverStep={null}
      pending={null}
      resolverSelection={resolverSelection}
      onAction={over.onAction ?? (() => {})}
      onOpenPath={() => {}}
    />,
  );
  return { ...result, model };
}

const pane = () => screen.getByTestId('sync-panel');
const paneTone = () => screen.getAllByTestId('chip')[0]?.getAttribute('data-tone');
const buttons = () => screen.getAllByRole('button').map((b) => b.textContent);

describe('SyncPanel', () => {
  it('says how far behind the branch is, and offers the merge', () => {
    mount({ session: null, drift: behindDrift });

    expect(pane()).toHaveAttribute('data-sync-state', 'behind');
    expect(paneTone()).toBe('cyan');
    expect(buttons()).toContain('Sync');
  });

  it("renders git's own words for a blocked sync, and no resolver", () => {
    mount({ session: session({ status: 'blocked', raw_error: 'fatal: could not read from remote' }) });

    expect(pane()).toHaveAttribute('data-sync-state', 'blocked');
    expect(paneTone()).toBe('amber');
    expect(screen.getByTestId('sync-raw-error')).toHaveTextContent(
      'fatal: could not read from remote',
    );
    expect(buttons()).not.toContain('Resolve with agent');
    expect(screen.queryByRole('combobox', { name: /harness/i })).toBeNull();
  });

  it('lists the unmerged paths, git’s words and the resolver on a conflict', () => {
    mount();

    expect(pane()).toHaveAttribute('data-sync-state', 'conflicted');
    expect(paneTone()).toBe('ruby');
    expect(screen.getByTestId('conflict-files')).toHaveTextContent('src/lib.rs');
    expect(screen.getByTestId('sync-raw-error')).toHaveTextContent(
      'CONFLICT (content): Merge conflict in src/lib.rs',
    );
    expect(screen.getByTestId('sync-worktree-path')).toHaveTextContent(
      '/repos/demeteo_wt_sync_feature-f-1',
    );
    expect(buttons()).toEqual(expect.arrayContaining(['Resolve with agent', 'Abort sync']));
  });

  it('sends a running resolution to the stream', () => {
    mount({ session: session({ status: 'resolving', user_may_intervene: false }) });

    expect(pane()).toHaveAttribute('data-sync-state', 'resolving');
    expect(paneTone()).toBe('violet');
    expect(buttons()).toContain('Open the stream');
  });

  it('offers the diff, the publish and the discard on a held resolution', () => {
    mount({ session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222' }) });

    expect(pane()).toHaveAttribute('data-sync-state', 'awaiting_review');
    expect(paneTone()).toBe('emerald');
    expect(buttons()).toEqual(
      expect.arrayContaining(['Review diff', 'Publish', 'Discard merge']),
    );
  });

  /** The buttons act on a worktree an agent is writing in. The backend says who
   *  owns the sync and the pane obeys rather than reading `status` itself. */
  it('offers nothing that writes while something else owns the sync', () => {
    mount({
      session: session({
        status: 'resolved',
        merge_commit_sha: 'c0ffeec2222',
        user_may_intervene: false,
      }),
    });

    for (const gone of ['Review diff', 'Publish', 'Discard merge', 'Abort sync']) {
      expect(buttons()).not.toContain(gone);
    }
  });

  it('hands the pressed intent back untranslated', async () => {
    const onAction = vi.fn();
    const userEvent = (await import('@testing-library/user-event')).default;
    mount({ onAction });

    await userEvent.click(screen.getByRole('button', { name: 'Resolve with agent' }));
    expect(onAction).toHaveBeenCalledWith('resolve');
  });
});
