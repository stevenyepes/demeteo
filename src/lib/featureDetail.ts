import { invoke } from "@tauri-apps/api/core";
import type { Machine, Project, RemoteRunMirror, StepExecution } from "../types";
import type { WorkflowDefinitionV2 } from "../components/canvas/types";

/** Wire shape of `feature_get_worktree` / `remote_get_worktree`. */
export interface FeatureWorktree {
  machine_id: string;
  worktree_path: string;
  branch: string;
  default_branch: string;
}

/**
 * The three fields the rerun harness picker reads off `get_agent_configs`.
 * Deliberately narrower than the settings tab's `AgentConfigView`: only
 * `enabled && available` decides whether a harness may be offered for a
 * retry, and widening it here would tie the run surface to the settings
 * module's shape.
 */
export interface AgentAvailability {
  kind: string;
  enabled: boolean;
  available: boolean;
}

/** Mirrors the Rust `MrInfo` returned by `publish_mr`. */
export interface MrInfo {
  url: string;
  state: string;
  number: number;
  provider_kind: string;
  provider_host: string;
}

/** Mirrors the Rust `CleanupResult` returned by `feature_cleanup`. */
export interface CleanupResult {
  policy: string;
  action: string;
  branch_deleted: boolean;
  row_deleted: boolean;
  mr_state: string | null;
  warnings: string[];
}

/** The persisted step rows for a feature's run, in `step_index` order. */
export async function listStepsForRun(featureId: string): Promise<StepExecution[]> {
  return invoke<StepExecution[]>("step_list_for_run", { featureId });
}

/** Every registered remote machine — used here only to resolve a display name. */
export async function listMachines(): Promise<Machine[]> {
  return invoke<Machine[]>("get_machines");
}

/** The project row behind a feature, or `null` when it no longer exists. */
export async function getProjectById(projectId: string): Promise<Project | null> {
  return invoke<Project | null>("get_project_by_id", { projectId });
}

/**
 * Which harnesses are installed *and* enabled on `machineId`. `refresh`
 * re-probes the machine instead of answering from the stored config.
 */
export async function listAgentConfigs(input: {
  machineId: string;
  refresh: boolean;
}): Promise<AgentAvailability[]> {
  return invoke<AgentAvailability[]>("get_agent_configs", {
    machineId: input.machineId,
    refresh: input.refresh,
  });
}

/**
 * The pinned version's schema-v2 graph, migrated backend-side. `null` for a
 * feature that has none — legacy runs started before workflows carried one.
 */
export async function getFeatureWorkflowGraph(
  featureId: string,
): Promise<WorkflowDefinitionV2 | null> {
  return invoke<WorkflowDefinitionV2 | null>("feature_workflow_graph", { featureId });
}

/**
 * The runner-owned shadow of this feature's run, or `null` when the feature
 * ran locally (it is not in the mirror at all).
 */
export async function remoteRunForFeature(
  featureId: string,
): Promise<RemoteRunMirror | null> {
  return invoke<RemoteRunMirror | null>("remote_run_for_feature", { featureId });
}

/** Re-hydrate a detached run's shadow from its runner over the tunnel. */
export async function remoteRefreshRun(input: {
  machineId: string;
  runId: string;
}): Promise<RemoteRunMirror | null> {
  return invoke<RemoteRunMirror | null>("remote_refresh_run", {
    machineId: input.machineId,
    runId: input.runId,
  });
}

/**
 * Cancel a detached run *on its runner*. The local `feature_cancel` has no
 * driver to signal for such a run — it would find no cancel sender and
 * return `Ok` having done nothing.
 */
export async function remoteCancelRun(input: {
  machineId: string;
  runId: string;
}): Promise<void> {
  await invoke<void>("remote_cancel_run", {
    machineId: input.machineId,
    runId: input.runId,
  });
}

/** Cancel a locally-driven run. Cancellation is feature-wide on both paths. */
export async function cancelFeature(featureId: string): Promise<void> {
  await invoke<void>("feature_cancel", { featureId });
}

/** Where a locally-driven (or attached-SSH) feature's worktree lives. */
export async function getFeatureWorktree(featureId: string): Promise<FeatureWorktree> {
  return invoke<FeatureWorktree>("feature_get_worktree", { featureId });
}

/**
 * Where a *detached* run's worktree lives: the runner is asked for its real
 * path and `machine_id` is re-homed onto the mirror's box, which the laptop
 * can already reach over SSH.
 */
export async function getRemoteWorktree(input: {
  machineId: string;
  runId: string;
}): Promise<FeatureWorktree> {
  return invoke<FeatureWorktree>("remote_get_worktree", {
    machineId: input.machineId,
    runId: input.runId,
  });
}

/**
 * Open a PR/MR for the feature branch. The backend supplies the title and
 * body — the agent's summary when the run wrote one, its own default when
 * it did not — so there is nothing for the caller to prompt for.
 */
export async function publishMr(input: {
  projectId: string;
  featureId: string;
  draft: boolean;
}): Promise<MrInfo> {
  return invoke<MrInfo>("publish_mr", {
    projectId: input.projectId,
    featureId: input.featureId,
    draft: input.draft,
  });
}

/**
 * Apply the project's `feature_lifecycle` policy (R6 decision 26).
 * `force` overrides the "the MR must be merged first" refusal on
 * `auto_delete`.
 */
export async function cleanupFeature(input: {
  featureId: string;
  force: boolean;
}): Promise<CleanupResult> {
  return invoke<CleanupResult>("feature_cleanup", {
    featureId: input.featureId,
    force: input.force,
  });
}
