import { invoke } from "@tauri-apps/api/core";
import type { EffortLevel, Feature, SyncOutcomeView, SyncSessionView } from "../types";

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
