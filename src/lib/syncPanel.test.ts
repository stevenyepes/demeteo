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
import type {
  DivergenceMove,
  FeatureDivergence,
  FeatureDrift,
  SyncSessionState,
  SyncSessionView,
} from '../types';

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
  blocked_stage: null,
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

/** The same count, off a ref the read could not refresh. `useFeatureDrift`
 *  always asks the pane's read to fetch, so this is a fetch that failed. */
const unfetched = (behind: number | null): FeatureDrift => ({
  ...drift(behind),
  fetched: false,
});

/** A branch that stopped on a divergence, which is one `blocked_stage` for
 *  both of the things that can be true of it. */
const divergedSession = (over: Partial<SyncSessionView> = {}): SyncSessionView =>
  session({
    status: 'blocked',
    blocked_stage: 'feature_diverged',
    conflict_files: [],
    raw_error: "'feature/f-1' has diverged from origin",
    ...over,
  });

const measured = (
  next_move: DivergenceMove,
  counts: { ahead?: number; behind?: number } = {},
): FeatureDivergence => ({ ahead: counts.ahead ?? 2, behind: counts.behind ?? 3, next_move });

/** `pending` defaults to "no call outstanding" and `divergence` to a branch
 *  nobody measured, which are the states every arm below is about; the ones
 *  that read either say so. */
const panel = (
  input: Omit<SyncPanelInput, 'pending' | 'divergence'> & {
    pending?: SyncIntent | null;
    divergence?: FeatureDivergence | null;
  },
) => describeSyncPanel({ pending: null, divergence: null, ...input });

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

  /**
   * The zero that is not an answer. A fetch that did not land leaves the
   * previous one's count in place, and settling on it is how the pane came to
   * say "Nothing to merge" beside a pull request the forge had already marked
   * conflicted. The sync fetches for itself, so the press is the way out.
   */
  it('keeps the sync on a zero the read could not verify', () => {
    const model = panel({ session: null, drift: unfetched(0), canSync: true });

    expect(model.state).not.toBe('up_to_date');
    expect(model.tone).toBe('slate');
    expect(model.headline).toContain('origin/master');
    expect(intents(model)).toEqual(['sync', 'refresh']);
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

  /** The section over `detail` names git, and a `verify` block stores the
   *  project's check harness saying so in its own words — a heading calling
   *  that git's is the pane telling the user the wrong thing ran. */
  it('names the harness, not git, over a transcript the harness wrote', () => {
    const checks = panel({
      session: session({ status: 'blocked', blocked_stage: 'verify' }),
      drift: null,
      canSync: true,
    });
    const push = panel({
      session: session({ status: 'blocked', blocked_stage: 'push' }),
      drift: null,
      canSync: true,
    });

    expect(checks.detailTitle).not.toMatch(/git/i);
    expect(push.detailTitle).toMatch(/git/i);
  });

  /** The stage is on the row since V46, and the two ends of it want opposite
   *  things said. A `push` failure has already committed the merge onto the
   *  feature branch, so the reassurance the other stages carry is false for it
   *  — and it is the one stage that must never be told "nothing was merged". */
  it('says which step stopped a blocked sync', () => {
    const stopped = panel({
      session: session({ status: 'blocked', blocked_stage: 'fetch', raw_error: 'fatal: could not read from remote' }),
      drift: null,
      canSync: true,
    });
    expect(stopped.headline).toMatch(/fetch/i);
    expect(stopped.body).toMatch(/nothing was merged/i);

    const held = panel({
      session: session({
        status: 'blocked',
        blocked_stage: 'push',
        merge_commit_sha: 'c0ffeec',
        raw_error: 'Sync merge succeeded locally but pushing to origin failed',
      }),
      drift: null,
      canSync: true,
    });
    expect(held.body).not.toMatch(/nothing was merged/i);
    expect(held.body).toMatch(/push/i);

    // `merge` is the stage that observed nothing at all about the branch: the
    // merge ran in a worktree with the feature branch checked out, so one that
    // committed before the channel dropped is already on it. The reassurance
    // the other stages carry is an assertion nobody made here.
    const cut = panel({
      session: session({ status: 'blocked', blocked_stage: 'merge' }),
      drift: null,
      canSync: true,
    });
    expect(cut.body).not.toMatch(/nothing reached the branch/i);
    expect(cut.body).not.toMatch(/nothing was merged/i);
  });

  /** The action a row that committed a merge actually needs. "Retry sync"
   *  merges nothing — the branch already contains the base — so it reports up
   *  to date and abandons the commit in a worktree the next sync force-removes.
   *  `verify` is the second such stage: the merge is clean and committed, and
   *  what withheld it was the project's own harness rather than the remote. */
  it('offers a sync that committed a merge the publish a retry would strand', () => {
    for (const stage of ['push', 'verify'] as const) {
      const model = panel({
        session: session({ status: 'blocked', blocked_stage: stage, merge_commit_sha: 'c0ffeec' }),
        drift: null,
        canSync: true,
      });

      expect(intents(model), stage).toContain('publish');
      expect(intents(model), stage).not.toContain('sync');
      // Nothing is conflicted in either, so the resolver would open a worktree
      // with nothing in it to resolve.
      expect(intents(model), stage).not.toContain('resolve');
      expect(model.headline, stage).not.toBe('');
    }
  });

  /** Every other stage merged nothing, so there is nothing of it to publish and
   *  a retry is exactly right. A row from before the column existed names no
   *  stage, and an unnamed stage may not be read as any particular one. */
  it('offers no publish for a blocked sync that merged nothing', () => {
    for (const stage of [
      'fetch',
      'base_ref_missing',
      'worktree_provision',
      'merge',
      'repo_context',
      'held_resolution',
      'turn_in_flight',
      null,
    ] as const) {
      const model = panel({
        session: session({ status: 'blocked', blocked_stage: stage }),
        drift: null,
        canSync: true,
      });
      expect(intents(model), `${stage}`).not.toContain('publish');
      expect(intents(model), `${stage}`).toContain('sync');
      expect(model.headline, `${stage}`).not.toBe('');
    }
  });

  /** The guard between a `push`-blocked row that recorded no commit and a
   *  Publish whose backend answer is a hard error. `publish` refuses without a
   *  sha, and the confirmation it would otherwise run — `git merge-base
   *  --is-ancestor <sha> origin/<branch>` — is refused outright by git for an
   *  empty one, so the user is told the push did not land about one that did.
   *  Both spellings of "no commit" have to withhold it: the column's own null,
   *  and the empty string an unread `rev-parse HEAD` used to be flattened to. */
  it('withholds publish from a merge-carrying sync that recorded no commit', () => {
    for (const sha of [null, ''] as const) {
      const model = panel({
        session: session({ status: 'blocked', blocked_stage: 'verify', merge_commit_sha: sha }),
        drift: null,
        canSync: true,
      });
      expect(intents(model), `${JSON.stringify(sha)}`).not.toContain('publish');
    }
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

  /** Every recovery affordance after a failed agent resolution. Asserted
   *  positively: the `user_may_intervene: false` case below asserts only
   *  absences, which a model offering nothing at all also satisfies.
   *
   *  `continue` is here because the refusal that produced this state names what
   *  is still conflicted — often very little — and the person reading it is
   *  entitled to finish that themselves rather than buy another turn. `watch`
   *  is here because the turn that failed is the thing they most want to read. */
  it('leaves a failed resolution every way out', () => {
    const model = panel({
      session: session({ status: 'resolution_failed', raw_error: 'agent exited 1' }),
      drift: null,
      canSync: true,
    });

    expect(model.state).toBe('resolution_failed');
    expect(model.tone).toBe('ruby');
    expect(model.showResolver).toBe(true);
    expect(intents(model)).toEqual(['resolve', 'continue', 'terminal', 'watch', 'abort']);
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
    expect(intents(model)).toEqual(['resolve', 'continue', 'terminal', 'abort']);
  });

  /** The pane has told people to resolve conflicts themselves in the worktree
   *  since it existed. Until `continue` there was nothing that would take the
   *  result: the only ways out were an agent or abandoning the sync. */
  it('offers the hand-resolved finish beside the agent on a fresh conflict', () => {
    const model = panel({ session: session(), drift: null, canSync: true });

    const finish = model.actions.find((action) => action.intent === 'continue');
    expect(finish?.label).toBe("I've resolved it");
    expect(finish?.desc).toContain('Spawns no agent');
  });

  /** Nothing that writes is offered on a sync somebody else is driving, and a
   *  hand-resolved finish writes as much as a resolution does. */
  it('offers no hand-resolved finish on a sync that is not the users', () => {
    const model = panel({
      session: session({ user_may_intervene: false }),
      drift: null,
      canSync: true,
    });

    expect(intents(model)).not.toContain('continue');
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

  /**
   * The session table holds one row per feature and `published_status` leaves a
   * published resolution on `resolved` forever, so this arm is where a feature
   * that ever hit a conflict spends the rest of its life. Answered as a dead
   * end it withheld Sync permanently while the header chip beside it went on
   * counting — and `resync_refusal` allows the sync all along.
   */
  it('offers the sync again once the base branch has moved past a published resolution', () => {
    const model = panel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', pushed_at: 1800 }),
      drift: drift(4),
      canSync: true,
    });

    expect(model.state).toBe('published');
    expect(model.chipLabel).toBe('4 behind');
    expect(model.tone).toBe(describeStaleness(drift(4))?.tone);
    expect(model.badge).toBe(4);
    expect(intents(model)).toEqual(['sync', 'refresh']);
  });

  /** A count that failed is not a count of zero — the same three-state rule
   *  `staleness.ts` opens on. A merge answers what the count could not. */
  it('offers the sync on a published resolution whose count could not be taken', () => {
    const model = panel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', pushed_at: 1800 }),
      drift: drift(null, null),
      canSync: true,
    });

    expect(intents(model)).toEqual(['sync', 'refresh']);
  });

  /** The published arm inherits the same rule, and needed saying twice: it is
   *  where a feature that ever hit a conflict spends the rest of its life. */
  it('keeps the sync on a published resolution whose zero could not be verified', () => {
    const model = panel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', pushed_at: 1800 }),
      drift: unfetched(0),
      canSync: true,
    });

    expect(model.tone).toBe('slate');
    expect(model.body).toContain('carries the merge');
    expect(intents(model)).toEqual(['sync', 'refresh']);
  });

  /** A verified zero is the one reading that settles it, and a run that has
   *  started writing to the branch again takes the press away regardless. */
  it('settles a published resolution the base branch has not moved past', () => {
    const even = panel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', pushed_at: 1800 }),
      drift: drift(0),
      canSync: true,
    });
    const running = panel({
      session: session({ status: 'resolved', merge_commit_sha: 'c0ffeec2222', pushed_at: 1800 }),
      drift: drift(4),
      canSync: false,
    });

    expect(even.tone).toBe('emerald');
    expect(intents(even)).toEqual(['refresh']);
    expect(intents(running)).toEqual(['refresh']);
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

  /**
   * The two refusals are one `blocked_stage` and differ only in their prose, so
   * what the pane offers is read from a measurement rather than sniffed out of
   * git's English. This is the arm `git cherry` settled as a rewrite: origin
   * carries every change the local commits make, so the reset drops no changes
   * — and only the commits, which is a loss a person has to accept.
   */
  it('offers both moves on a branch origin rewrote, the destructive one in ruby', () => {
    const model = panel({
      session: divergedSession(),
      drift: null,
      divergence: measured('reset_onto_origin', { ahead: 2, behind: 3 }),
      canSync: true,
    });

    expect(model.state).toBe('blocked');
    expect(intents(model)).toEqual(['reconcile', 'reset_onto_origin', 'refresh', 'abort']);
    expect(model.actions.find((action) => action.intent === 'reconcile')?.tone).toBe('violet');
    expect(model.actions.find((action) => action.intent === 'reset_onto_origin')?.tone).toBe('ruby');
    expect(model.headline).toMatch(/rewrote/i);
    expect(model.body).toContain('3 commits');
    expect(model.body).toContain('2 commits');
  });

  /** Disjoint work has no reset to offer: nothing has proved origin carries
   *  these commits' changes, and the merge is the move that can lose neither
   *  side. The counts are said in the pane's own words, not git's. */
  it('offers only the merge where the two sides are different work', () => {
    const model = panel({
      session: divergedSession(),
      drift: null,
      divergence: measured('merge_origin', { ahead: 1, behind: 1 }),
      canSync: true,
    });

    expect(intents(model)).toEqual(['reconcile', 'refresh', 'abort']);
    expect(model.headline).toMatch(/never had/i);
    expect(model.body).toContain('1 commit this checkout does not');
    expect(model.body).not.toMatch(/1 commits/);
  });

  /** `refuse` is `git cherry` saying it cannot tell — a partial rewrite, or a
   *  read that failed — and an absent measurement is the same non-answer. Both
   *  keep the refusal that hands the decision back, retry included. */
  it('keeps the standing refusal for a divergence nothing could classify', () => {
    for (const divergence of [null, measured('refuse')]) {
      const model = panel({
        session: divergedSession(),
        drift: null,
        divergence,
        canSync: true,
      });

      const said = `${divergence?.next_move ?? 'unread'}`;
      expect(intents(model), said).toEqual(['sync', 'refresh', 'abort']);
      expect(model.headline, said).toBe('This branch and origin have both moved');
    }
  });

  /** A reading describes the two refs and nothing else, so it may only answer
   *  the stage that stopped on them: a fetch that failed says nothing about
   *  what this branch may do about origin. */
  it('reconciles nothing on a block that is not the divergence', () => {
    const model = panel({
      session: session({ status: 'blocked', blocked_stage: 'fetch' }),
      drift: null,
      divergence: measured('reset_onto_origin'),
      canSync: true,
    });

    expect(intents(model)).toEqual(['sync', 'refresh', 'abort']);
    expect(model.headline).toMatch(/fetch/i);
  });

  /** Both presses move the branch, so both obey what the rest of the pane obeys
   *  — the backend's ownership flag, and a run that is still committing here.
   *  What was measured is a fact about the branch either way, so the copy
   *  stays. */
  it('offers no reconcile on a divergence that is not the user’s to move', () => {
    const theirs = panel({
      session: divergedSession({ user_may_intervene: false }),
      drift: null,
      divergence: measured('reset_onto_origin'),
      canSync: true,
    });
    const running = panel({
      session: divergedSession(),
      drift: null,
      divergence: measured('reset_onto_origin'),
      canSync: false,
    });

    for (const model of [theirs, running]) {
      expect(intents(model)).not.toContain('reconcile');
      expect(intents(model)).not.toContain('reset_onto_origin');
      expect(model.headline).toMatch(/rewrote/i);
    }
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
  it.each<SyncIntent>(['sync', 'resolve', 'abort', 'publish', 'discard', 'reconcile', 'reset_onto_origin'])(
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

  it.each<SyncIntent>([
    'sync',
    'resolve',
    'abort',
    'publish',
    'discard',
    'refresh',
    'reconcile',
    'reset_onto_origin',
  ])(
    'holds %s back while another call is out',
    (intent) => {
      expect(isReadOnlySyncIntent(intent)).toBe(false);
    },
  );
});
