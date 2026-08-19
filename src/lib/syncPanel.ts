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
 * it is the only thing this reads.
 *
 * Drift decides the copy only in the arms where no sync is live. `up_to_date`,
 * `merged` and `aborted` sessions describe a sync that is over; what the user
 * needs then is the count, and `describeStaleness` is already the authority for
 * both its wording and its tone — resolving a second one here is how the header
 * chip and this pane would come to disagree about the same branch.
 */

import type { ConflictFile, FeatureDrift, SyncSessionView } from '../types';
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

export type SyncIntent =
  | 'sync'
  | 'resolve'
  | 'abort'
  | 'review'
  | 'publish'
  | 'discard'
  | 'refresh'
  | 'watch';

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
  /** Whether a sync may be started at all: a run still committing to this
   *  branch would be merging into its own working tree. */
  canSync: boolean;
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
export function describeSyncPanel({ session, drift, canSync }: SyncPanelInput): SyncPanelModel {
  const base: Omit<SyncPanelModel, 'state' | 'tone' | 'chipLabel' | 'headline' | 'body'> = {
    live: false,
    detail: null,
    conflictFiles: [],
    showResolver: false,
    showTelemetry: false,
    worktreePath: session?.worktree_path ?? null,
    branch: session?.feature_branch ?? null,
    baseBranch: session?.base_branch ?? null,
    actions: [],
    badge: 0,
  };

  if (session === null || session.status === 'up_to_date' || session.status === 'merged' || session.status === 'aborted') {
    return { ...base, ...quiet(drift, canSync) };
  }

  const mine = session.user_may_intervene;
  const branch = session.feature_branch;
  const baseBranch = session.base_branch;

  switch (session.status) {
    case 'syncing':
      return {
        ...base,
        state: 'syncing',
        tone: 'cyan',
        chipLabel: 'Syncing',
        live: true,
        headline: `Merging ${baseBranch} into ${branch}`,
        body: 'The merge is running. What it finds — a clean merge, a conflict, or a reason it could not start — lands here.',
      };

    case 'blocked':
      return {
        ...base,
        state: 'blocked',
        tone: 'amber',
        chipLabel: 'Blocked',
        headline: 'The sync stopped before git reached a verdict',
        body: 'Nothing was merged and nothing is conflicted, so there is nothing for an agent to do. git said why below; fix that and sync again.',
        detail: session.raw_error,
        actions: canSync ? [{ ...SYNC, label: 'Retry sync', tone: 'amber' }, REFRESH] : [REFRESH],
        badge: 1,
      };

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
        actions: mine ? [WATCH, ABORT] : [WATCH],
      };

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
            tone: 'emerald',
            chipLabel: 'Not published',
            headline: 'Conflicts resolved, not published',
            body: `The merge is on ${branch} and origin has not seen it. Read it before it reaches the pull request.`,
            conflictFiles: session.conflict_files,
            showTelemetry: true,
            actions: mine ? reviewActions(session) : [],
            badge: 1,
          }
        : {
            ...base,
            state: 'published',
            tone: 'emerald',
            chipLabel: 'Published',
            headline: 'The resolution is on origin',
            body: `${branch} carries the merge and origin has it. Nothing here is waiting on you.`,
            actions: [REFRESH],
          };
  }
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
    headline: chip.label === '1 behind' ? '1 commit is missing from this branch' : `${behind} commits are missing from this branch`,
    body: chip.title,
    actions: [SYNC, REFRESH],
    badge: behind,
  };
}
