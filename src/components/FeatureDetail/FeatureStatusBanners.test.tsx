/**
 * The review card's render gate — the one surface that can end an unpublished
 * resolution, and the only thing standing between a merge nobody has read and
 * the open pull request.
 *
 * It went in unguarded: inverting the condition, so the card rendered only for
 * resolutions already on origin and never for the one it exists to show, left
 * tsc and every test green.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { SyncSessionView } from '../../types';
import { FeatureStatusBanners } from './FeatureStatusBanners';

const session = (over: Partial<SyncSessionView> = {}): SyncSessionView => ({
  feature_id: 'f-1',
  machine_id: 'local',
  repo_dir: '/repos/demeteo',
  feature_branch: 'feature/f-1',
  base_branch: 'master',
  status: 'resolved',
  worktree_path: '/repos/demeteo_wt_sync_feature-f-1',
  head_before: 'aaaaaaa1111',
  merge_commit_sha: 'c0ffeec2222',
  conflict_files: [],
  raw_error: null,
  pushed_at: null,
  user_may_intervene: true,
  attempts: 0,
  created_at: 0,
  updated_at: 0,
  ...over,
});

function mount(syncSession: SyncSessionView | null) {
  const noop = () => {};
  return render(
    <FeatureStatusBanners
      status="completed"
      syncBanner={null}
      resolving={false}
      aborting={false}
      onResolveConflicts={noop}
      resolverSelection={
        {
          agentKind: '',
          model: '',
          effort: '',
          setAgentKind: noop,
          setModel: noop,
          setEffort: noop,
        } as never
      }
      onAbortSync={noop}
      onDismissSyncBanner={noop}
      syncSession={syncSession}
      reviewPending={null}
      onViewSyncDiff={noop}
      onPublishSync={noop}
      onDiscardSync={noop}
      mrUrl={null}
      mrState={null}
      onRefreshMrState={noop}
    />,
  );
}

describe('FeatureStatusBanners', () => {
  it('offers the review for a resolution that is committed and not on origin', () => {
    mount(session());
    expect(screen.getByTestId('sync-review')).toBeInTheDocument();
  });

  it('offers nothing once the resolution has reached origin', () => {
    mount(session({ pushed_at: 1700 }));
    expect(screen.queryByTestId('sync-review')).toBeNull();
  });

  /** The buttons act on a branch a driver still holds; the backend says so and
   *  the card obeys rather than reading `status` for itself. */
  it('offers nothing while something else still owns the sync', () => {
    mount(session({ user_may_intervene: false }));
    expect(screen.queryByTestId('sync-review')).toBeNull();
  });

  it('offers nothing for a sync with no resolution on it', () => {
    for (const status of ['conflicted', 'merged', 'aborted', 'blocked'] as const) {
      const { unmount } = mount(session({ status }));
      expect(screen.queryByTestId('sync-review')).toBeNull();
      unmount();
    }
    mount(null);
    expect(screen.queryByTestId('sync-review')).toBeNull();
  });
});
