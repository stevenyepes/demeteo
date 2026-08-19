/**
 * The Sync pane's whole policy, asserted without a DOM.
 *
 * Tones are read from the shared vocabulary rather than spelled: a model that
 * answers `'amber'` where `runStatus.ts` says otherwise passes a literal
 * assertion while re-opening audit F27, and this is the one file that decides
 * the mapping for every sync state.
 */
import { describe, expect, it } from 'vitest';

import { describeSyncPanel } from './syncPanel';
import { describeStaleness } from './staleness';
import type { FeatureDrift, SyncSessionState, SyncSessionView } from '../types';

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
  raw_error: 'CONFLICT (content): Merge conflict in src/lib.rs',
  pushed_at: null,
  user_may_intervene: true,
  attempts: 1,
  created_at: 0,
  updated_at: 0,
  ...over,
});

const drift = (behind: number | null, ahead: number | null = 2): FeatureDrift => ({
  divergence: { behind, ahead },
  base_ref: 'origin/master',
  fetched: true,
  checked_at: 0,
});

const intents = (model: ReturnType<typeof describeSyncPanel>) =>
  model.actions.map((action) => action.intent);

describe('describeSyncPanel', () => {
  it('reads a behind branch off the drift count, in the tone the chip already uses', () => {
    const model = describeSyncPanel({ session: null, drift: drift(3), canSync: true });

    expect(model.state).toBe('behind');
    expect(model.tone).toBe(describeStaleness(drift(3))?.tone);
    expect(model.chipLabel).toBe('3 behind');
    expect(model.badge).toBe(3);
    expect(intents(model)).toContain('sync');
  });

  it('puts no count on a branch that is level with its base', () => {
    const model = describeSyncPanel({ session: null, drift: drift(0), canSync: true });

    expect(model.state).toBe('up_to_date');
    expect(model.badge).toBe(0);
    expect(intents(model)).not.toContain('sync');
  });

  /** "We could not count it" and "there is nothing to pull" are different
   *  answers, and collapsing them is the reason nothing could say a branch was
   *  behind before `lib/staleness.ts`. */
  it('keeps an unmeasurable branch out of the up-to-date arm', () => {
    const model = describeSyncPanel({ session: null, drift: drift(null, null), canSync: true });

    expect(model.state).toBe('unknown');
    expect(model.tone).toBe('slate');
    expect(intents(model)).toContain('sync');
  });

  it('offers no sync while the run is still committing to the branch', () => {
    const model = describeSyncPanel({ session: null, drift: drift(4), canSync: false });

    expect(model.state).toBe('run_active');
    expect(model.actions).toEqual([]);
  });

  it('offers a retry and no resolver for a blocked sync', () => {
    const model = describeSyncPanel({
      session: session({ status: 'blocked', raw_error: 'fatal: could not read from remote' }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('blocked');
    expect(model.tone).toBe('amber');
    expect(model.detail).toBe('fatal: could not read from remote');
    expect(intents(model)).toContain('sync');
    expect(intents(model)).not.toContain('resolve');
    expect(model.showResolver).toBe(false);
  });

  it('counts the conflicted files and offers the resolver against them', () => {
    const model = describeSyncPanel({ session: session(), drift: null, canSync: true });

    expect(model.state).toBe('conflicted');
    expect(model.tone).toBe('ruby');
    expect(model.chipLabel).toBe('2 conflicts');
    expect(model.badge).toBe(2);
    expect(model.showResolver).toBe(true);
    expect(model.worktreePath).toBe('/repos/demeteo_wt_sync_feature-f-1');
    expect(intents(model)).toEqual(['resolve', 'abort']);
  });

  it('sends a running resolution to the stream rather than to a button', () => {
    const model = describeSyncPanel({
      session: session({ status: 'resolving', user_may_intervene: false }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('resolving');
    expect(model.tone).toBe('violet');
    expect(model.live).toBe(true);
    expect(intents(model)).toEqual(['watch']);
  });

  it('offers the review, the publish and the discard on a held resolution', () => {
    const model = describeSyncPanel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222' }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('awaiting_review');
    expect(model.tone).toBe('emerald');
    expect(intents(model)).toEqual(['review', 'publish', 'discard']);
    expect(model.badge).toBe(1);
  });

  /** A resolution already on origin is finished. Offering to publish it again
   *  is the notice that makes a user think the first press failed. */
  it('stops offering anything once the resolution is on origin', () => {
    const model = describeSyncPanel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', pushed_at: 1800 }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('published');
    expect(intents(model)).toEqual(['refresh']);
  });

  /** The pre-merge tip is unrecoverable once the merge lands, so a session
   *  that never recorded one has no honest base to diff or reset against. */
  it('offers no diff and no discard without a recorded pre-merge tip', () => {
    const model = describeSyncPanel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', head_before: null }),
      drift: null,
      canSync: true,
    });

    expect(intents(model)).toEqual(['publish']);
  });

  describe('a sync something else is driving', () => {
    const owned: SyncSessionState[] = ['conflicted', 'resolving', 'resolved', 'resolution_failed'];

    it.each(owned)('offers %s nothing that writes', (status) => {
      const model = describeSyncPanel({
        session: session({
          status,
          merge_commit_sha: 'c0ffeec2222',
          user_may_intervene: false,
        }),
        drift: null,
        canSync: true,
      });

      for (const forbidden of ['resolve', 'abort', 'publish', 'discard', 'review']) {
        expect(intents(model)).not.toContain(forbidden);
      }
      expect(model.showResolver).toBe(false);
    });
  });
});
