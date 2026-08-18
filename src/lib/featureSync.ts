import { invoke } from "@tauri-apps/api/core";
import type { Feature, SyncOutcomeView } from "../types";

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
 * Spawn a fresh agent session dedicated to resolving the merge
 * conflicts left by `syncFeature`. The agent edits the conflict
 * files in a temporary worktree, commits the resolution, and the
 * worktree is merged back into the feature branch.
 */
export async function resolveSyncConflicts(
  featureId: string,
  conflictFiles: string[],
): Promise<SyncOutcomeView> {
  return invoke<SyncOutcomeView>("feature_resolve_sync_conflicts", {
    featureId,
    conflictFiles,
  });
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
