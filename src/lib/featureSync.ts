import { invoke } from "@tauri-apps/api/core";
import type {
  EffortLevel,
  Feature,
  FeatureDrift,
  SyncOutcomeView,
  SyncResolverView,
  SyncSessionView,
} from "../types";

/**
 * Sync the feature branch with `origin/<default_branch>`. Returns
 * a tagged result:
 *
 * - `{ status: "ok" }` when the merge was clean.
 * - `{ status: "conflict" }` when the merge left unmerged paths; the
 *   conflict files are in `conflict_files`. This is the only outcome
 *   that may offer "Resolve with agent" — `resolveSyncConflicts`
 *   takes the same file list.
 * - `{ status: "blocked" }` when the sync stopped short of a merge
 *   verdict. Nothing is known to be conflicted, so there is nothing for
 *   an agent to do; `stage` says what to fix and `raw_error` is git's
 *   own words.
 * - `{ status: "resolved" }` after a successful agent resolution.
 * - `{ status: "resolution_failed" }` when the agent could not
 *   clean up the conflicts.
 */
export async function syncFeature(featureId: string): Promise<SyncOutcomeView> {
  return invoke<SyncOutcomeView>("feature_sync", { featureId });
}

/**
 * How far the feature branch has fallen behind the base a sync would merge —
 * the reason to press "Sync with main", answered without merging anything.
 *
 * `refresh` fetches `origin/<base>` first. Left off, the counts are as of
 * whenever that ref last moved; the answer's `fetched` says which it was, so a
 * caller never presents a week-old number as this minute's.
 */
export async function getFeatureDrift(
  featureId: string,
  refresh = false,
): Promise<FeatureDrift> {
  return invoke<FeatureDrift>("feature_drift", { featureId, refresh });
}

/**
 * What one resolution attempt asks to be run under. Every field `null` means
 * "inherit", which is the request this function sent before there was a picker:
 * the backend falls through the project's conflict-resolver default, the
 * harness the run was launched with, and then the project default.
 */
export interface SyncResolverChoice {
  agentKind: string | null;
  model: string | null;
  effort: EffortLevel | null;
}

/**
 * Who a resolution would run under with every field of the choice left
 * `null` — the label the picker shows for "Inherit", read from the same chain
 * the resolve call walks rather than guessed from the feature.
 */
export async function getSyncResolver(featureId: string): Promise<SyncResolverView> {
  return invoke<SyncResolverView>("feature_sync_resolver", { featureId });
}

/**
 * Spawn a fresh agent session dedicated to resolving the merge
 * conflicts left by `syncFeature`. The agent edits the conflict
 * files in a temporary worktree, commits the resolution, and the
 * worktree is merged back into the feature branch.
 */
export async function resolveSyncConflicts(
  featureId: string,
  conflictFiles: string[],
  resolver?: SyncResolverChoice,
): Promise<SyncOutcomeView> {
  return invoke<SyncOutcomeView>("feature_resolve_sync_conflicts", {
    featureId,
    conflictFiles,
    agentKind: resolver?.agentKind ?? null,
    model: resolver?.model ?? null,
    effort: resolver?.effort ?? null,
  });
}

/**
 * The feature's live sync, or `null` when it has never synced.
 *
 * Reconciled against the working tree by the backend before it answers, so a
 * `conflicted` session is one git still agrees with rather than a row nothing
 * has revisited since a process died.
 */
export async function getSyncSession(
  featureId: string,
): Promise<SyncSessionView | null> {
  return invoke<SyncSessionView | null>("sync_session_get", { featureId });
}

/**
 * Give up on the feature's sync: undo the merge, discard the sync worktree and
 * close the session. Safe when the worktree is already gone.
 */
export async function abortSync(
  featureId: string,
): Promise<SyncSessionView | null> {
  return invoke<SyncSessionView | null>("sync_abort", { featureId });
}

/**
 * Publish a resolution that is only on the feature branch.
 *
 * Safe to press twice: a resolution already on origin answers with itself
 * rather than pushing again. The backend confirms the push against the
 * remote-tracking ref before recording it, so a rejected or unfinished push
 * comes back as an error and the session stays unpublished.
 */
export async function publishSyncResolution(
  featureId: string,
): Promise<SyncSessionView | null> {
  return invoke<SyncSessionView | null>("sync_publish", { featureId });
}

/**
 * Throw a resolution away: move the feature branch back to where the merge
 * found it and abandon the sync.
 *
 * What comes back is an abandoned sync, **not** the conflict — reproducing that
 * would mean re-running the merge against an origin that has moved since. A
 * session that never recorded its pre-merge tip is refused rather than reset to
 * a guess.
 */
export async function discardSyncResolution(
  featureId: string,
): Promise<SyncSessionView | null> {
  return invoke<SyncSessionView | null>("sync_discard", { featureId });
}

/**
 * Whether this session holds a resolution that is committed on the branch,
 * has not reached origin, and is the user's to act on — the one state the
 * review card exists for, and the one the "resolved" success banner must be
 * suppressed under so nobody reads it as shipped.
 *
 * One predicate because it is asserted in three places that must not drift:
 * the card's render gate, the banner's suppression, and whether "Sync with
 * main" is still offered. `user_may_intervene` is the backend's answer and not
 * re-derived here; it is what keeps a resolution a live run still owns out of
 * all three.
 */
export function isAwaitingSyncReview(session: SyncSessionView | null): boolean {
  return (
    session !== null &&
    session.status === 'resolved' &&
    session.pushed_at === null &&
    session.user_may_intervene
  );
}

/**
 * Refresh the MR state on a feature. Hits the provider's HTTP API
 * (GitHub or GitLab) and returns the latest `mr_state`. The caller
 * is expected to persist the result back to the feature row.
 */
export async function fetchMrState(
  projectId: string,
  mrUrl: string,
): Promise<string> {
  return invoke<string>("fetch_mr_state", { projectId, mrUrl });
}

/** Lightweight `feature_get` wrapper that returns `null` on 404. */
export async function getFeature(featureId: string): Promise<Feature | null> {
  return invoke<Feature | null>("feature_get", { featureId });
}

/**
 * The `step_id` an out-of-band sync records itself under
 * (`domain::step_seed::MANUAL_SYNC_STEP_ID`).
 *
 * The row exists so the resolution can stream to an id the inspector
 * subscribes to, and it lands in `step_executions` beside the run's own rows —
 * which is also the frontend's rollup input. Nothing in the row marks it as
 * out-of-band, so every reader that summarises a *run* has to exclude it: a
 * manual sync the user tried once and gave up on would otherwise report a
 * finished run as failed forever, and the actions the inspector offers a node
 * are refused for it by the backend.
 */
export const MANUAL_SYNC_STEP_ID = "s-sync-manual";

/** Whether `stepId` names a row no workflow node produced. */
export function isOutOfBandStep(stepId: string): boolean {
  return stepId === MANUAL_SYNC_STEP_ID;
}
