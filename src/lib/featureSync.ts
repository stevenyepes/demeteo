import { invoke } from "@tauri-apps/api/core";
import type { Feature, SyncOutcomeView } from "../types";

/**
 * Sync the feature branch with `origin/<default_branch>`. Returns
 * a tagged result:
 *
 * - `{ status: "ok" }` when the merge was clean.
 * - `{ status: "conflict" }` when conflicts were detected; the
 *   conflict files are in `conflict_files`. The UI surfaces a
 *   "Resolve with agent" button that calls
 *   `resolveSyncConflicts` with the same file list.
 * - `{ status: "resolved" }` after a successful agent resolution.
 * - `{ status: "resolution_failed" }` when the agent could not
 *   clean up the conflicts.
 *
 * `revalidateStepExecutionId` is optional: when provided, the named
 * step is replayed after a successful sync so the workflow re-runs
 * validation on the freshly merged tree.
 */
export async function syncFeature(
  featureId: string,
  revalidateStepExecutionId?: string | null,
): Promise<SyncOutcomeView> {
  return invoke<SyncOutcomeView>("feature_sync", {
    featureId,
    revalidateStepExecutionId: revalidateStepExecutionId ?? null,
  });
}

/**
 * Spawn a fresh agent session dedicated to resolving the merge
 * conflicts left by `syncFeature`. The agent edits the conflict
 * files in a temporary worktree, commits the resolution, and the
 * worktree is merged back into the feature branch. If
 * `revalidateStepExecutionId` is set, the named step is replayed
 * so the workflow re-runs validation on the freshly merged tree.
 */
export async function resolveSyncConflicts(
  featureId: string,
  conflictFiles: string[],
  revalidateStepExecutionId?: string | null,
): Promise<SyncOutcomeView> {
  return invoke<SyncOutcomeView>("feature_resolve_sync_conflicts", {
    featureId,
    conflictFiles,
    revalidateStepExecutionId: revalidateStepExecutionId ?? null,
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
