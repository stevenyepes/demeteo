/**
 * What the Sync pane says, decided here so it can be asserted without
 * rendering anything — the frontend counterpart of AGENTS.md §3's rule that a
 * policy decision may not be spelled inside a function that also does I/O.
 *
 * **Everything is read from the durable session, never from the outcome of the
 * call that produced it.** Five separate surfaces grew here, each remembering
 * its own half of a sync in a `useState`, and a conflict therefore existed only
 * for as long as the component that had watched it happen stayed mounted. The
 * row is the only thing that survives a navigation, a remount or a restart, so
 * it is the only thing this reads. `pending` is the one input that is not the
 * row, and it says which call is outstanding rather than what any call found —
 * see its own note for why the two live arms need it.
 *
 * Drift decides the copy only in the arms where no sync is live. `up_to_date`,
 * `merged` and `aborted` sessions describe a sync that is over; what the user
 * needs then is the count, and `describeStaleness` is already the authority for
 * both its wording and its tone — resolving a second one here is how the header
 * chip and this pane would come to disagree about the same branch.
 */

import type {
  ConflictFile,
  DivergenceMove,
  FeatureDivergence,
  FeatureDrift,
  SyncBlockedStage,
  SyncSessionView,
} from '../types';
import type { RunStatusTone } from './runStatus';
import { describeStaleness } from './staleness';

export type SyncPanelState =
  | 'run_active'
  | 'unknown'
  | 'up_to_date'
  | 'behind'
  | 'syncing'
  | 'blocked'
  | 'conflicted'
  | 'resolving'
  | 'awaiting_review'
  | 'published'
  | 'resolution_failed';

/**
 * The two reconciles are two intents and not one carrying a move, because the
 * intent is what a row is keyed, labelled and disabled by and what `pending`
 * names: one value shared by both presses puts "Merging…" on the button that
 * resets.
 */
export type SyncIntent =
  | 'sync'
  | 'resolve'
  | 'abort'
  | 'review'
  | 'publish'
  | 'discard'
  | 'refresh'
  | 'watch'
  | 'reconcile'
  | 'reset_onto_origin';

/** Tones an action row may take. A subset of `RunStatusTone`: `slate` is what
 *  an inert row looks like, and nothing offering a press is inert. */
export type SyncActionTone = 'violet' | 'cyan' | 'amber' | 'ruby' | 'emerald';

export interface SyncAction {
  intent: SyncIntent;
  label: string;
  tone: SyncActionTone;
  title: string;
  desc: string;
}

export interface SyncPanelModel {
  state: SyncPanelState;
  tone: RunStatusTone;
  chipLabel: string;
  /** The chip's dot pulses: something is moving without anybody pressing. */
  live: boolean;
  headline: string;
  body: string;
  /** git's own words, for a `<pre>`. */
  detail: string | null;
  /** What wrote `detail`. A `verify` block stores the project's check harness
   *  output, not git's, and the section over it said "What git said" — over a
   *  transcript naming a command the user recognises as their own. */
  detailTitle: string;
  conflictFiles: ConflictFile[];
  /** The harness picker belongs to a state that can actually spawn a resolver. */
  showResolver: boolean;
  /** Whether the resolver's own step row has anything to report yet. */
  showTelemetry: boolean;
  worktreePath: string | null;
  branch: string | null;
  baseBranch: string | null;
  actions: SyncAction[];
  /** Count for the pane's tab. 0 renders none. */
  badge: number;
}

export interface SyncPanelInput {
  session: SyncSessionView | null;
  drift: FeatureDrift | null;
  /**
   * What the branch and `origin/<feature>` each hold that the other does not,
   * for a sync that stopped on the divergence, or `null`.
   *
   * The second input that is not the row, and for the same reason as `pending`
   * (`DivergenceMove` in `src/types.ts`). It is read rather than parsed out of
   * `raw_error` — both refusals are one `blocked_stage`, and telling them apart
   * by their English is a pane deciding a git question from prose.
   */
  divergence: FeatureDivergence | null;
  /** Whether a sync may be started at all: a run still committing to this
   *  branch would be merging into its own working tree. */
  canSync: boolean;
  /**
   * The intent whose call has not answered yet, or `null`.
   *
   * Not an exception to reading the durable row. `feature_sync` opens a
   * `syncing` row *before* the merge and `feature_resolve_sync_conflicts` moves
   * the row to `resolving` at the top of the turn, and both answer only once the
   * whole thing is over — so for the length of a merge the row already says what
   * these two arms say, and the one thing missing is that anybody re-read it.
   * What is local here is that a call is outstanding, never what it found; the
   * verdict still comes from the row. Without it the pane spent every merge it
   * started saying "Nothing has been read yet".
   */
  pending: SyncIntent | null;
}

/**
 * Whether an intent in flight can move the branch a drift count describes.
 *
 * The count is suspended while one can, because a number taken between a merge
 * and its push describes neither side. `refresh` is the one intent that cannot:
 * it *is* the count, and suspending the read on it is how the pane's own
 * Refresh came to pay for a `git fetch` and then throw the answer away.
 */
export function syncIntentMovesBranch(intent: SyncIntent | null): boolean {
  if (intent === null || intent === 'refresh') return false;
  return !isReadOnlySyncIntent(intent);
}

/**
 * Whether an intent reaches for nothing the backend can be mid-way through.
 *
 * These two select a step row and open a read-only diff, so another sync action
 * being in flight is no reason to refuse them — and `watch` in particular is
 * offered *only* while a resolver is running, which is exactly when a blanket
 * "another sync action is still running" would take it away.
 */
export function isReadOnlySyncIntent(intent: SyncIntent): boolean {
  return intent === 'watch' || intent === 'review';
}

const REFRESH: SyncAction = {
  intent: 'refresh',
  label: 'Refresh',
  tone: 'cyan',
  title: 'Fetch the base branch and count again',
  desc: 'Nothing else in the app moves that ref for a finished feature, so the count only changes when you ask.',
};

const SYNC: SyncAction = {
  intent: 'sync',
  label: 'Sync',
  tone: 'violet',
  title: 'Merge the base branch into this feature branch',
  desc: 'Merges the base branch in. A clean merge finishes here; a conflicted one comes back to this pane.',
};

const ABORT: SyncAction = {
  intent: 'abort',
  label: 'Abort sync',
  tone: 'ruby',
  title: 'Undo the merge and discard the sync worktree',
  desc: 'Undoes the merge and discards the sync worktree. The branch goes back to where the sync found it.',
};

const RESOLVE: SyncAction = {
  intent: 'resolve',
  label: 'Resolve with agent',
  tone: 'violet',
  title: 'Spawn an agent in the sync worktree to clean up the markers',
  desc: 'Spawns a fresh agent in the sync worktree to clean up the conflict markers and commit the result.',
};

/**
 * Abort, worded for a blocked sync.
 *
 * `ABORT`'s copy promises the branch goes back to where the sync found it,
 * which is true of an open merge and false of the one blocked stage that
 * persists work: a failed *push* has already committed the merge onto the
 * feature branch, and `close_session` undoes the merge inside the throwaway
 * tree and removes it without moving a single ref.
 */
const ABANDON: SyncAction = {
  intent: 'abort',
  label: 'Abandon sync',
  tone: 'ruby',
  title: 'Discard the sync worktree and close this session',
  desc: 'Discards the sync worktree and closes the session. Anything git already committed to the branch stays on it.',
};

const WATCH: SyncAction = {
  intent: 'watch',
  label: 'Open the stream',
  tone: 'cyan',
  title: "Select the resolver's step and show its live output",
  desc: "Switches to the step pane on the resolver's own row, where its output streams live.",
};

/**
 * Every state's model. One `switch` over what the row says, each arm returning
 * the facts its copy needs and nothing else.
 *
 * Actions that write are gated on `user_may_intervene` and **omitted**, not
 * disabled: a session a run's own sync step is driving belongs to that turn —
 * abort would delete a directory an agent is writing in, resolve would put a
 * second agent in the same tree. A disabled button invites a press and then
 * explains; an absent one says the sync is not yours. The flag is the backend's
 * answer and is never re-derived from `status` here, because the window where
 * the row still reads `conflicted` while a step owns it is exactly what it
 * closes.
 */
export function describeSyncPanel({
  session,
  drift,
  divergence,
  canSync,
  pending,
}: SyncPanelInput): SyncPanelModel {
  const base: PanelBase = {
    live: false,
    detail: null,
    detailTitle: 'What git said',
    conflictFiles: [],
    showResolver: false,
    showTelemetry: false,
    worktreePath: session?.worktree_path ?? null,
    branch: session?.feature_branch ?? null,
    baseBranch: session?.base_branch ?? null,
    actions: [],
    badge: 0,
  };

  if (pending === 'sync') {
    return syncingArm(base, session?.base_branch ?? null, session?.feature_branch ?? null);
  }
  if (pending === 'resolve' && session !== null) {
    return resolvingArm(base, session);
  }

  if (session === null || session.status === 'up_to_date' || session.status === 'merged' || session.status === 'aborted') {
    return { ...base, ...quiet(drift, canSync) };
  }

  const mine = session.user_may_intervene;
  const branch = session.feature_branch;
  const baseBranch = session.base_branch;

  switch (session.status) {
    case 'syncing':
      return syncingArm(base, baseBranch, branch);

    case 'blocked':
      return blockedArm(base, session, divergence, canSync, mine);

    case 'conflicted':
      return {
        ...base,
        state: 'conflicted',
        tone: 'ruby',
        chipLabel: conflictLabel(session.conflict_files.length),
        headline: `Merge conflict on ${branch}`,
        body: `The merge left unmerged paths in the sync worktree. Resolve them yourself there, or spawn an agent to do it. Either way the merge stays on disk until one of you finishes it.`,
        detail: session.raw_error,
        conflictFiles: session.conflict_files,
        showResolver: mine,
        actions: mine ? [RESOLVE, ABORT] : [],
        badge: Math.max(session.conflict_files.length, 1),
      };

    case 'resolving':
      return resolvingArm(base, session);

    case 'resolution_failed':
      return {
        ...base,
        state: 'resolution_failed',
        tone: 'ruby',
        chipLabel: 'Resolver failed',
        headline: 'The agent could not clear the conflict',
        body: 'The merge is still on disk with its markers. Try another harness, finish it by hand in the worktree, or abort the sync.',
        detail: session.raw_error,
        conflictFiles: session.conflict_files,
        showResolver: mine,
        showTelemetry: true,
        actions: mine ? [{ ...RESOLVE, label: 'Try again' }, ABORT] : [],
        badge: 1,
      };

    case 'resolved':
      return session.pushed_at === null
        ? {
            ...base,
            state: 'awaiting_review',
            // Amber, not emerald. `runStatus.ts` fixes the vocabulary — emerald
            // is "done well", amber is "needs a human" — and the whole point of
            // this state is that the merge is on the branch, origin has not seen
            // it, and nothing publishes it without a press.
            tone: 'amber',
            chipLabel: 'Not published',
            headline: 'Conflicts resolved, not published',
            body: `The merge is on ${branch} and origin has not seen it. Read it before it reaches the pull request.`,
            conflictFiles: session.conflict_files,
            showTelemetry: true,
            actions: mine ? reviewActions(session) : [],
            badge: 1,
          }
        : publishedArm(base, session, drift, canSync);
  }
}

/**
 * A resolution origin already has, which is a sync that is over — and the one
 * finished sync whose row never says so.
 *
 * `published_status` (domain/sync_session.rs) leaves a published resolution on
 * `resolved` on purpose, so the row that reaches this arm looks nothing like
 * the `up_to_date`, `merged` and `aborted` rows the header of this module
 * hands to `quiet`. Answered as a dead end it took Sync away from the feature
 * for good: the session table holds one row per feature, that row stays here
 * forever, and the base branch goes on moving underneath it — the pane saying
 * "nothing here is waiting on you" beside a header chip counting four behind,
 * with `resync_refusal` having allowed the sync the whole time.
 *
 * So the landed fact is a sentence, not a state: `quiet` stays the only place
 * that turns a count into copy, a tone and a press.
 */
function publishedArm(
  base: PanelBase,
  session: SyncSessionView,
  drift: FeatureDrift | null,
  canSync: boolean,
): SyncPanelModel {
  const landed = `${session.feature_branch} carries the merge and origin has it.`;
  const chip = describeStaleness(drift);
  const behind = drift?.divergence.behind ?? null;
  // `behind === null` is a count that could not be taken, and it earns the
  // press for the reason `quiet` gives it: a merge answers what the count
  // could not. Only a measured zero settles this arm.
  if (!canSync || chip === null || behind === 0) {
    return {
      ...base,
      state: 'published',
      tone: 'emerald',
      chipLabel: 'Published',
      headline: 'The resolution is on origin',
      body: `${landed} Nothing here is waiting on you.`,
      actions: [REFRESH],
    };
  }
  return {
    ...base,
    state: 'published',
    tone: chip.tone,
    chipLabel: chip.label,
    headline: behind === null ? 'The count could not be taken' : missingHeadline(behind),
    body: `${landed} ${chip.title}`,
    actions: [SYNC, REFRESH],
    badge: behind ?? 0,
  };
}

type PanelBase = Omit<SyncPanelModel, 'state' | 'tone' | 'chipLabel' | 'headline' | 'body'>;

/**
 * A merge that is running. Reached from the stored `syncing` row and from an
 * unanswered `sync` call, which describe the same thing from either side of the
 * IPC boundary.
 *
 * `Refresh` is the only affordance, and it is not decoration: nothing polls this
 * row, and while the merge is running every intervention is refused for it, so
 * without a press that re-reads the row a merge that finished is visible only by
 * navigating away and back. A press is also what retires a `syncing` row left
 * behind by a restart — the read reconciles it to `blocked`, which is a state
 * the user can finally clear.
 */
function syncingArm(base: PanelBase, baseBranch: string | null, branch: string | null): SyncPanelModel {
  return {
    ...base,
    state: 'syncing',
    tone: 'cyan',
    chipLabel: 'Syncing',
    live: true,
    headline: baseBranch && branch ? `Merging ${baseBranch} into ${branch}` : 'Merging the base branch in',
    body: 'The merge is running. What it finds — a clean merge, a conflict, or a reason it could not start — lands here.',
    actions: [REFRESH],
  };
}

/**
 * What each blocked stage means for the person reading it, which is not one
 * sentence but one per stage.
 *
 * The stage lives on the row since V46. Before that this arm knew only that
 * *something* stopped, so its copy had to be true of a fetch that moved no git
 * object and of a push that had already committed the merge — which left it
 * saying almost nothing, and offering "Retry sync" to both.
 */
const BLOCKED_COPY: Record<SyncBlockedStage, { headline: string; body: string }> = {
  fetch: {
    headline: 'The base branch could not be fetched',
    body: 'Nothing was merged and nothing moved. The remote is unreachable, or the stored credentials no longer open it.',
  },
  base_ref_missing: {
    headline: 'The base branch is not on the remote',
    body: 'The fetch worked; the branch this run is based on does not exist upstream. Nothing was merged — check the run\u2019s base branch.',
  },
  worktree_provision: {
    headline: 'The sync worktree could not be created',
    body: 'The merge never started, so nothing was merged. git\u2019s reason is below.',
  },
  merge: {
    headline: 'The merge stopped without a verdict',
    // No claim about the branch. The two sentences used to end "Nothing
    // reached the branch", which is the one thing this stage means nobody
    // observed: the merge ran in a worktree with the feature branch checked
    // out, so a merge that completed and then lost its channel has already
    // committed there. Abandoning reclaims the tree and a retry then merges
    // nothing, so the reassurance was pointing at the case that loses work.
    body: 'It was cut short rather than answered, so what the sync worktree holds is unknown — possibly clean, possibly half-applied, possibly a merge that committed before the connection went. Check the branch before retrying; abandoning reclaims the worktree either way.',
  },
  push: {
    headline: 'The merge is on the branch and origin has not seen it',
    body: 'The merge succeeded and is committed; only the push failed. Syncing again would merge nothing and leave it here — publish it instead.',
  },
  verify: {
    headline: 'The merge is committed and the checks failed in it',
    // No suggestion that anything is conflicted. git merges text, so two edits
    // that never share a line merge cleanly and can still leave a tree that
    // does not build — nothing here has unmerged paths, and the resolver
    // `blocked_refusal` withholds would open a worktree with nothing in it to
    // resolve.
    body: 'The merge is clean and committed; the project\u2019s own checks then went red in it, so it was not pushed. Fix it on the branch and publish, or publish it anyway and let CI say the same thing.',
  },
  feature_diverged: {
    headline: 'This branch and origin have both moved',
    // The divergence `git cherry` could not settle: a partial rewrite, or a
    // read that failed. It offers no retry — that counts the same two
    // histories and refuses again — and no merge either, though a merge is
    // what the classified divergences get: where the two sides are only half
    // the same work, which history the branch is meant to be is a person's
    // answer. `divergedCopy` has the two that were read.
    body: 'Origin has commits on this branch that this checkout does not, and this checkout has commits origin does not. Nothing was merged: the base would have gone onto a branch that is missing origin\u2019s work. Push or reset the branch to reconcile the two, then sync.',
  },
  repo_context: {
    headline: 'This feature has no repository to sync',
    body: 'No git command was ever issued. The project\u2019s repository row could not be resolved.',
  },
  held_resolution: {
    headline: 'A resolution is still waiting to be read',
    body: 'The last sync left a merge on this branch that nobody has published or discarded, so no new sync was started.',
  },
  turn_in_flight: {
    headline: 'Another sync is already running on this feature',
    body: 'Nothing was merged: the sync worktree belongs to the turn that already holds it. Wait for that one to finish, or stop it.',
  },
};

const UNNAMED_BLOCK = {
  headline: 'The sync stopped short of a verdict',
  body: "Nothing is conflicted, so there is nothing for an agent to do. This row predates the column that records which step stopped, so read git\u2019s words below before retrying.",
};

/**
 * Publish, worded for a merge that never got past its own push.
 *
 * `reviewActions`\u2019 Publish sits beside a diff of a *resolution*; this one
 * publishes a clean merge nobody resolved anything in, and the reason to press
 * it is that the alternative — a retry — quietly does nothing: the branch
 * already contains the base, so the second merge changes nothing, reports up to
 * date, and abandons the commit here.
 */
const PUBLISH_BLOCKED: SyncAction = {
  intent: 'publish',
  label: 'Publish merge',
  tone: 'emerald',
  title: 'Push the merge that already landed on this branch',
  desc: 'Pushes the merge the sync already committed. Safe to press twice — one already on origin answers with itself.',
};

/**
 * A sync that stopped, said in the stage\u2019s own terms.
 *
 * `push` and `verify` are the stages with work at risk, and they are the two
 * that do not offer a retry: the merge is already committed on the branch, so a
 * second sync finds the base merged, changes nothing, and leaves the commit
 * sitting in a worktree the sync after it force-removes. What separates them is
 * only *who* withheld the push — the remote, or the project\u2019s own harness —
 * which is a difference in copy, not in what the row may be offered.
 */
function blockedArm(
  base: PanelBase,
  session: SyncSessionView,
  divergence: FeatureDivergence | null,
  canSync: boolean,
  mine: boolean,
): SyncPanelModel {
  const stage = session.blocked_stage;
  const held = stage === 'push' || stage === 'verify';
  const measured = measuredDivergence(stage, divergence);
  const copy = measured === null ? blockedCopy(stage) : divergedCopy(measured);
  const reconcile: SyncAction[] =
    measured !== null && canSync && mine
      ? reconcileActions(session.feature_branch, measured.next_move)
      : [];
  // No retry beside a reconcile: it is the one press that re-measures the same
  // two histories and stops on the same sentence, and first in the list it
  // reads as the recommended one.
  const retry: SyncAction[] =
    !held && canSync && reconcile.length === 0
      ? [{ ...SYNC, label: 'Retry sync', tone: 'amber' as const }]
      : [];
  // A sha this row does not carry is the whole of what stands between Publish
  // and a press that cannot succeed: `publish` refuses without one, and the
  // confirmation it would otherwise run is `git merge-base --is-ancestor <sha>`,
  // which git rejects outright for an empty argument — so the user is told the
  // push did not land, forever, about one that did.
  const named = (session.merge_commit_sha ?? '') !== '';
  const publish: SyncAction[] = held && mine && named ? [PUBLISH_BLOCKED] : [];
  return {
    ...base,
    state: 'blocked',
    tone: 'amber',
    chipLabel: held ? 'Not published' : 'Blocked',
    headline: copy.headline,
    body: copy.body,
    detail: session.raw_error,
    detailTitle: stage === 'verify' ? 'What the checks said' : base.detailTitle,
    actions: [...publish, ...reconcile, ...retry, REFRESH, ...(mine ? [ABANDON] : [])],
    badge: 1,
  };
}

function blockedCopy(stage: SyncBlockedStage | null): { headline: string; body: string } {
  return stage === null ? UNNAMED_BLOCK : BLOCKED_COPY[stage];
}

/**
 * The divergence this row is allowed to be read against, or `null`.
 *
 * Two guards, each closing a way the pane would speak for a measurement it does
 * not have: a reading only ever describes the stage that stopped on it, and a
 * `refuse` is `git cherry` saying it cannot tell — the same non-answer as no
 * reading at all, and the arm that offers nothing.
 */
function measuredDivergence(
  stage: SyncBlockedStage | null,
  divergence: FeatureDivergence | null,
): FeatureDivergence | null {
  if (stage !== 'feature_diverged' || divergence === null) return null;
  return divergence.next_move === 'refuse' ? null : divergence;
}

/**
 * A divergence that was classified, which is a different thing to say than the
 * refusal above it: both are the same `blocked_stage`, and what separates them
 * is that `git cherry` could tell whether origin's side is other work or this
 * checkout's own work rewritten.
 */
function divergedCopy(divergence: FeatureDivergence): { headline: string; body: string } {
  const { ahead, behind } = divergence;
  if (divergence.next_move === 'reset_onto_origin') {
    return {
      headline: 'Origin rewrote this branch',
      body: `Origin has ${commits(behind)} this checkout does not, and every one of this checkout’s ${commits(ahead)} is already in them under a different sha — what a rebase, a squash or an amend somewhere else looks like from here. Nothing was merged. Resetting onto origin drops no changes, only the local commits that carry them; merging origin in keeps both histories.`,
    };
  }
  return {
    headline: 'Origin has work on this branch that this checkout has never had',
    body: `Origin has ${commits(behind)} this checkout does not, and this checkout has ${commits(ahead)} origin does not — different work on each side, none of it the same change twice. Nothing was merged. Merging origin in loses neither side; sync again after it to bring the base branch in.`,
  };
}

/**
 * The presses a classified divergence earns, which is the merge always and the
 * reset only where the rewrite was actually read.
 *
 * That asymmetry is the whole point of measuring: a merge cannot drop either
 * side, so it needs no evidence about the other one, while the reset abandons
 * the local commits and is offered only where `git cherry` proved origin
 * already carries every change they make. Proving content is as far as it goes
 * — whether those commits were meant to survive is not in the history — which
 * is why the reset is ruby and a press rather than something a sync does.
 */
function reconcileActions(branch: string, move: DivergenceMove): SyncAction[] {
  const merge: SyncAction = {
    intent: 'reconcile',
    label: 'Merge origin into this branch',
    tone: 'violet',
    title: `Merge origin/${branch} into ${branch}`,
    desc: 'Writes a merge that keeps both sides. Nothing is dropped on either — sync again afterwards to bring the base branch in.',
  };
  if (move !== 'reset_onto_origin') return [merge];
  return [
    merge,
    {
      intent: 'reset_onto_origin',
      label: 'Reset onto origin',
      tone: 'ruby',
      title: `Move ${branch} to origin/${branch}`,
      desc: 'Moves the branch onto origin and abandons the local commits. Origin already carries their changes; what goes is the commits themselves.',
    },
  ];
}

/**
 * An agent in the sync worktree. Reached from the stored `resolving` row and
 * from an unanswered `resolve` call: `resolve_sync_conflicts` moves the row to
 * `resolving` at the top of the turn and answers only when the turn is over, so
 * the pane spent the whole resolution rendering the `conflicted` row it started
 * from — ruby chip, "N conflicts", no telemetry and no way to reach the stream.
 */
function resolvingArm(base: PanelBase, session: SyncSessionView): SyncPanelModel {
  return {
    ...base,
    state: 'resolving',
    tone: 'violet',
    chipLabel: 'Resolving',
    live: true,
    headline: 'An agent is resolving the conflict',
    body: 'It is editing the conflict files in the sync worktree and will commit the result. Its output streams on the step pane.',
    conflictFiles: session.conflict_files,
    showTelemetry: true,
    actions: session.user_may_intervene ? [WATCH, ABORT] : [WATCH],
  };
}

/**
 * The diff a resolution is reviewed as is `head_before..merge_commit_sha`, and
 * nothing else. `merge_commit_sha^` names the pre-merge tip only while the
 * resolution is a single merge commit; a resolver that added a follow-up commit
 * — which agents do routinely — makes the first parent that follow-up's parent
 * instead, and the review then silently omits the merge itself. The pre-merge
 * tip is persisted at sync time (V43 `head_before`) precisely because it is
 * unrecoverable afterwards, so a session lacking it offers no diff rather than
 * one against a guess.
 */
function reviewActions(session: SyncSessionView): SyncAction[] {
  const reviewable = session.head_before !== null && session.merge_commit_sha !== null;
  const actions: SyncAction[] = [];
  if (reviewable) {
    actions.push({
      intent: 'review',
      label: 'Review diff',
      tone: 'violet',
      title: `Diff ${session.head_before?.slice(0, 7)}..${session.merge_commit_sha?.slice(0, 7)}`,
      desc: 'Opens the read-only editor on the merge alone — what the resolution changed, not what the branch did.',
    });
  }
  if (session.merge_commit_sha !== null) {
    actions.push({
      intent: 'publish',
      label: 'Publish',
      tone: 'emerald',
      title: 'Push the resolution to origin',
      desc: 'Pushes the resolution to origin. Safe to press twice — one already there answers with itself.',
    });
  }
  if (reviewable) {
    actions.push({
      intent: 'discard',
      label: 'Discard merge',
      tone: 'ruby',
      title: `Move ${session.feature_branch} back to ${session.head_before?.slice(0, 7)}`,
      desc: 'Moves the branch back to where the merge found it and abandons the sync. The conflict is not restored — sync again for a fresh one.',
    });
  }
  return actions;
}

function conflictLabel(count: number): string {
  if (count === 0) return 'Conflicted';
  return count === 1 ? '1 conflict' : `${count} conflicts`;
}

type Quiet = Pick<SyncPanelModel, 'state' | 'tone' | 'chipLabel' | 'headline' | 'body' | 'actions' | 'badge'>;

function quiet(drift: FeatureDrift | null, canSync: boolean): Quiet {
  if (!canSync) {
    return {
      state: 'run_active',
      tone: 'slate',
      chipLabel: 'Not yet',
      headline: 'This branch is still being written',
      body: 'The run is still committing here. A count taken now describes neither side of the merge, so syncing waits until it stops.',
      actions: [],
      badge: 0,
    };
  }

  const chip = describeStaleness(drift);
  if (chip === null) {
    return {
      state: 'unknown',
      tone: 'slate',
      chipLabel: 'Checking',
      headline: 'Counting this branch against its base',
      body: 'Nothing has been read yet.',
      actions: [],
      badge: 0,
    };
  }

  const behind = drift?.divergence.behind ?? null;
  if (behind === null) {
    return {
      state: 'unknown',
      tone: chip.tone,
      chipLabel: chip.label,
      headline: 'The count could not be taken',
      body: `${chip.title} Syncing is still offered — a merge answers the question the count could not.`,
      actions: [SYNC, REFRESH],
      badge: 0,
    };
  }
  if (behind === 0) {
    return {
      state: 'up_to_date',
      tone: chip.tone,
      chipLabel: chip.label,
      headline: 'Nothing to merge',
      body: chip.title,
      actions: [REFRESH],
      badge: 0,
    };
  }
  return {
    state: 'behind',
    tone: chip.tone,
    chipLabel: chip.label,
    headline: missingHeadline(behind),
    body: chip.title,
    actions: [SYNC, REFRESH],
    badge: behind,
  };
}

/** Read by both arms that offer a Sync, so a count cannot be worded one way
 *  under a fresh branch and another under a published resolution. */
function missingHeadline(behind: number): string {
  return behind === 1
    ? '1 commit is missing from this branch'
    : `${behind} commits are missing from this branch`;
}

function commits(count: number): string {
  return count === 1 ? '1 commit' : `${count} commits`;
}
