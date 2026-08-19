/**
 * The Sync pane's whole policy, asserted without a DOM.
 *
 * Tones are read from the shared vocabulary rather than spelled: a model that
 * answers `'amber'` where `runStatus.ts` says otherwise passes a literal
 * assertion while re-opening audit F27, and this is the one file that decides
 * the mapping for every sync state.
 */
import { describe, expect, it } from 'vitest';

import {
  describeSyncPanel,
  isReadOnlySyncIntent,
  syncIntentMovesBranch,
  type SyncIntent,
  type SyncPanelInput,
} from './syncPanel';
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

/** `pending` defaults to "no call outstanding", which is the state every arm
 *  below is about; the two that read it say so. */
const panel = (input: Omit<SyncPanelInput, 'pending'> & { pending?: SyncIntent | null }) =>
  describeSyncPanel({ pending: null, ...input });

const intents = (model: ReturnType<typeof describeSyncPanel>) =>
  model.actions.map((action) => action.intent);

describe('describeSyncPanel', () => {
  it('reads a behind branch off the drift count, in the tone the chip already uses', () => {
    const model = panel({ session: null, drift: drift(3), canSync: true });

    expect(model.state).toBe('behind');
    expect(model.tone).toBe(describeStaleness(drift(3))?.tone);
    expect(model.chipLabel).toBe('3 behind');
    expect(model.badge).toBe(3);
    expect(intents(model)).toContain('sync');
  });

  it('puts no count on a branch that is level with its base', () => {
    const model = panel({ session: null, drift: drift(0), canSync: true });

    expect(model.state).toBe('up_to_date');
    expect(model.badge).toBe(0);
    expect(intents(model)).not.toContain('sync');
  });

  /** "We could not count it" and "there is nothing to pull" are different
   *  answers, and collapsing them is the reason nothing could say a branch was
   *  behind before `lib/staleness.ts`. */
  it('keeps an unmeasurable branch out of the up-to-date arm', () => {
    const model = panel({ session: null, drift: drift(null, null), canSync: true });

    expect(model.state).toBe('unknown');
    expect(model.tone).toBe('slate');
    expect(intents(model)).toContain('sync');
  });

  it('offers no sync while the run is still committing to the branch', () => {
    const model = panel({ session: null, drift: drift(4), canSync: false });

    expect(model.state).toBe('run_active');
    expect(model.actions).toEqual([]);
  });

  it('offers a retry and no resolver for a blocked sync', () => {
    const model = panel({
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

  /** `SyncBlockedStage::Push` persists as `Blocked` and has already committed
   *  the merge onto the feature branch (`adapters/merge.rs`), so a body claiming
   *  nothing was merged is false in exactly the stage that leaves work behind —
   *  and `stage` is not persisted, so this arm cannot tell which one it has. */
  it('does not tell a blocked sync that nothing was merged', () => {
    const model = panel({
      session: session({ status: 'blocked', raw_error: 'Sync merge succeeded locally but pushing to origin failed' }),
      drift: null,
      canSync: true,
    });

    expect(model.body).not.toMatch(/nothing was merged/i);
    expect(model.body).toMatch(/push/i);
  });

  /** The backend permits `Abort` for a `Blocked` session and the row names the
   *  worktree precisely so `sync_abort` can reclaim it; the pane rendered the
   *  path and offered nothing that removed it. */
  it('offers a blocked sync a way to reclaim the worktree it names', () => {
    const model = panel({
      session: session({ status: 'blocked' }),
      drift: null,
      canSync: true,
    });

    expect(intents(model)).toContain('abort');
    const abort = model.actions.find((action) => action.intent === 'abort');
    // Not the shared `ABORT` copy: a failed push already moved the branch, so
    // "the branch goes back to where the sync found it" would be a lie.
    expect(abort?.desc).not.toMatch(/back to where the sync found it/i);
  });

  it('offers no abort on a blocked sync somebody else is driving', () => {
    const model = panel({
      session: session({ status: 'blocked', user_may_intervene: false }),
      drift: null,
      canSync: true,
    });

    expect(intents(model)).not.toContain('abort');
  });

  /** Nothing polls this row, `reconcile` passes `Syncing` through untouched and
   *  every intervention is refused for it — so a row left behind by an
   *  interrupted sync had no press at all, and a merge that finished was
   *  visible only by navigating away and back. */
  it('says a merge is running, and leaves one press that can re-read the row', () => {
    const model = panel({
      session: session({ status: 'syncing', conflict_files: [], raw_error: null }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('syncing');
    expect(model.tone).toBe('cyan');
    expect(model.chipLabel).toBe('Syncing');
    expect(model.live).toBe(true);
    expect(model.headline).toBe('Merging origin/master into feature/f-1');
    expect(intents(model)).toEqual(['refresh']);
  });

  /** `feature_sync` opens a `syncing` row before the merge and answers only
   *  once the whole thing is over, and nothing re-reads the row in between — so
   *  the pane spent every merge it started saying nothing had been read yet. */
  it('renders the merge it started rather than an unread branch', () => {
    const model = panel({ session: null, drift: null, canSync: true, pending: 'sync' });

    expect(model.state).toBe('syncing');
    expect(model.live).toBe(true);
    expect(model.headline).not.toMatch(/counting/i);
  });

  /** Same window on the resolve side: the row moves to `resolving` at the top
   *  of the turn while the call runs to the end of it, so the pane kept the
   *  ruby "N conflicts" chip for the whole resolution and `Open the stream` —
   *  offered only here — was unreachable exactly while there was a stream. */
  it('renders the resolution it started rather than the conflict it started from', () => {
    const model = panel({ session: session(), drift: null, canSync: true, pending: 'resolve' });

    expect(model.state).toBe('resolving');
    expect(model.tone).toBe('violet');
    expect(model.live).toBe(true);
    expect(model.showTelemetry).toBe(true);
    expect(intents(model)).toContain('watch');
  });

  /** Both recovery affordances after a failed agent resolution. Asserted
   *  positively: the `user_may_intervene: false` case below asserts only
   *  absences, which a model offering nothing at all also satisfies. */
  it('leaves a failed resolution both ways out', () => {
    const model = panel({
      session: session({ status: 'resolution_failed', raw_error: 'agent exited 1' }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('resolution_failed');
    expect(model.tone).toBe('ruby');
    expect(model.showResolver).toBe(true);
    expect(intents(model)).toEqual(['resolve', 'abort']);
    expect(model.actions[0]?.label).toBe('Try again');
  });

  it('counts the conflicted files and offers the resolver against them', () => {
    const model = panel({ session: session(), drift: null, canSync: true });

    expect(model.state).toBe('conflicted');
    expect(model.tone).toBe('ruby');
    expect(model.chipLabel).toBe('2 conflicts');
    expect(model.badge).toBe(2);
    expect(model.showResolver).toBe(true);
    expect(model.worktreePath).toBe('/repos/demeteo_wt_sync_feature-f-1');
    expect(intents(model)).toEqual(['resolve', 'abort']);
  });

  it('sends a running resolution to the stream rather than to a button', () => {
    const model = panel({
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
    const model = panel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222' }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('awaiting_review');
    // Amber, not emerald: `runStatus.ts` reserves emerald for "done well", and
    // this merge is on the branch with origin unaware of it and nothing that
    // publishes it without a press.
    expect(model.tone).toBe('amber');
    expect(intents(model)).toEqual(['review', 'publish', 'discard']);
    expect(model.badge).toBe(1);
  });

  /** A resolution already on origin is finished. Offering to publish it again
   *  is the notice that makes a user think the first press failed. */
  it('stops offering anything once the resolution is on origin', () => {
    const model = panel({
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
    const model = panel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', head_before: null }),
      drift: null,
      canSync: true,
    });

    expect(intents(model)).toEqual(['publish']);
  });

  describe('a sync something else is driving', () => {
    const owned: SyncSessionState[] = ['conflicted', 'resolving', 'resolved', 'resolution_failed'];

    it.each(owned)('offers %s nothing that writes', (status) => {
      const model = panel({
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

describe('syncIntentMovesBranch', () => {
  /** The drift read is suspended while one of these is in flight. `refresh`
   *  *is* the read, and suspending on it superseded the fetch that press had
   *  just paid for, landing the cached count instead. */
  it.each<SyncIntent>(['sync', 'resolve', 'abort', 'publish', 'discard'])(
    'suspends the count while %s is in flight',
    (intent) => {
      expect(syncIntentMovesBranch(intent)).toBe(true);
    },
  );

  it.each<SyncIntent | null>([null, 'refresh', 'watch', 'review'])(
    'leaves the count running for %s',
    (intent) => {
      expect(syncIntentMovesBranch(intent)).toBe(false);
    },
  );
});

describe('isReadOnlySyncIntent', () => {
  /** `watch` is offered only while a resolver is running, which is exactly when
   *  a blanket "another sync action is still running" would disable it. */
  it.each<SyncIntent>(['watch', 'review'])('keeps %s pressable during another call', (intent) => {
    expect(isReadOnlySyncIntent(intent)).toBe(true);
  });

  it.each<SyncIntent>(['sync', 'resolve', 'abort', 'publish', 'discard', 'refresh'])(
    'holds %s back while another call is out',
    (intent) => {
      expect(isReadOnlySyncIntent(intent)).toBe(false);
    },
  );
});
