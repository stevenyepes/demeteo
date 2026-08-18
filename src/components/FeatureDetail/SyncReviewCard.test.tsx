import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { SyncSessionView } from '../../types';
import { SyncReviewCard } from './SyncReviewCard';

/**
 * The one assertion this file exists for is the diff's *base*.
 *
 * `merge_commit_sha^` is the tempting spelling and it reads correctly for a
 * resolution that is exactly one merge commit. Agents routinely add a follow-up
 * commit, and then the first parent is that commit's parent — so the review
 * shows a diff that omits the merge it was opened to check, with nothing on
 * screen to say so. The pre-merge tip is persisted (`head_before`, V43) because
 * it is the only base that survives that, and a session without one has to say
 * the base is unknown rather than substitute a guess.
 */
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

describe('SyncReviewCard', () => {
  it('diffs the tip the sync recorded, not the merge commit’s first parent', async () => {
    const onViewDiff = vi.fn();
    render(
      <SyncReviewCard
        session={session()}
        pending={null}
        onViewDiff={onViewDiff}
        onPush={() => {}}
        onDiscard={() => {}}
      />,
    );

    await userEvent.click(screen.getByRole('button', { name: /view diff/i }));

    expect(onViewDiff).toHaveBeenCalledWith({
      baseRef: 'aaaaaaa1111',
      headRef: 'c0ffeec2222',
    });
  });

  it('offers no diff and no discard when the pre-merge tip was never recorded', () => {
    const onViewDiff = vi.fn();
    render(
      <SyncReviewCard
        session={session({ head_before: null })}
        pending={null}
        onViewDiff={onViewDiff}
        onPush={() => {}}
        onDiscard={() => {}}
      />,
    );

    expect(screen.getByRole('button', { name: /view diff/i })).toBeDisabled();
    expect(screen.getByRole('button', { name: /discard merge/i })).toBeDisabled();
    // Publishing stays available: the resolution is on the branch either way,
    // and the missing base only costs the review, not the merge.
    expect(screen.getByRole('button', { name: /push to origin/i })).toBeEnabled();
  });

  it('says the merge is gone, never that the conflict comes back', () => {
    render(
      <SyncReviewCard
        session={session()}
        pending={null}
        onViewDiff={() => {}}
        onPush={() => {}}
        onDiscard={() => {}}
      />,
    );

    const discard = screen.getByRole('button', { name: /discard merge/i });
    expect(discard.getAttribute('title')).toContain('abandon this sync');
    expect(discard.getAttribute('title')).toContain('conflict is not restored');
  });
});
