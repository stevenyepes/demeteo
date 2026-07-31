import { invoke } from "@tauri-apps/api/core";
import type { RemoteRunMirror, RunEvent } from "../types";

export async function listMirroredRuns(): Promise<RemoteRunMirror[]> {
  return invoke<RemoteRunMirror[]>("remote_list_mirrored_runs");
}

/** Re-poll every tracked runner and return the refreshed mirrors. Also what
 *  raises the OS notification for a run that turned actionable while the
 *  laptop was away, so this is not interchangeable with
 *  {@link listMirroredRuns}. */
export async function reconcileRuns(): Promise<RemoteRunMirror[]> {
  return invoke<RemoteRunMirror[]>("remote_reconcile_runs");
}

/** What `remote_submit_run` resolves with — the Rust `RemoteRunHandle`. */
export interface RemoteRunHandle {
  run_id: string;
  machine_id: string;
  status: string;
  feature_id: string;
}

/**
 * Hand a run to a machine's runner. The shadow Feature named by
 * `feature_id` is inserted before the RPC, so it exists even if the
 * submission then fails — the reconcile loop, not this call, is what
 * hydrates it.
 *
 * `targetRepoId` is singular by design: a detached run clones exactly one
 * repository, unlike a local run.
 */
export async function submitRemoteRun(input: {
  machineId: string;
  projectId: string;
  workflowId: string;
  title: string;
  description: string;
  agentKind: string | null;
  model: string | null;
  effort: string | null;
  commitArtifacts: boolean | null;
  loopIterations: number | null;
  maxBudgetUsd: number | null;
  stepOverrides: unknown[] | null;
  stagedAttachments: unknown[] | null;
  targetRepoId: string | null;
  unattended: boolean;
  maxCostUsd: number | null;
  maxWallClockSecs: number | null;
}): Promise<RemoteRunHandle> {
  return invoke<RemoteRunHandle>("remote_submit_run", { ...input });
}

/** The mirror row for a feature, or `null` when the feature ran locally. */
export async function remoteRunForFeature(featureId: string): Promise<RemoteRunMirror | null> {
  return invoke<RemoteRunMirror | null>("remote_run_for_feature", { featureId });
}

/** Provider compare/tree URL for a branch a run pushed, or `null` when no
 *  repo/provider resolves for it — a missing deep link, not an error. */
export async function remoteRunDiffUrl(
  projectId: string,
  branch: string,
): Promise<string | null> {
  return invoke<string | null>("remote_run_diff_url", { projectId, branch });
}

/**
 * The runner's own live status document. Returned as `unknown` because the
 * command forwards the runner's JSON verbatim: the laptop and the runner are
 * separately versioned, so the payload's shape is the runner's promise, not
 * this build's. Read it through a narrowing helper such as
 * {@link parkedGateId}.
 */
export async function getRemoteRunStatus(machineId: string, runId: string): Promise<unknown> {
  return invoke<unknown>("remote_get_status", { machineId, runId });
}

/** The step-execution id of the gate this run is parked on, if any. `null`
 *  covers both "not parked" and "parked with nothing to decide" — an
 *  over-budget park has no gate. */
export function parkedGateId(status: unknown): string | null {
  if (status == null || typeof status !== "object") return null;
  const value = (status as { parked_gate_id?: unknown }).parked_gate_id;
  return typeof value === "string" ? value : null;
}

/** Decide a gate on the runner. `approve | redirect | cancel` mean exactly
 *  what they mean locally — the RPC delegates to the same `GatePresenter`. */
export async function decideRemoteGate(input: {
  machineId: string;
  runId: string;
  gateId: string;
  decision: string;
  feedback: string | null;
}): Promise<void> {
  return invoke<void>("remote_decide_gate", {
    machineId: input.machineId,
    runId: input.runId,
    gateId: input.gateId,
    decision: input.decision,
    feedback: input.feedback,
  });
}

/** Re-send the run's git credential over the tunnel. The runner holds it in
 *  memory only, so a runner restart leaves the run waiting for the laptop. */
export async function reinjectRemoteCredentials(
  machineId: string,
  runId: string,
): Promise<RemoteRunMirror | null> {
  return invoke<RemoteRunMirror | null>("remote_reinject_credentials", { machineId, runId });
}

export async function cancelRemoteRun(machineId: string, runId: string): Promise<void> {
  return invoke<void>("remote_cancel_run", { machineId, runId });
}

/** Tail the run's append-only event log from `fromOffset` (M3.3). */
export async function streamRemoteEvents(
  machineId: string,
  runId: string,
  fromOffset: number,
): Promise<RunEvent[]> {
  return invoke<RunEvent[]>("remote_stream_events", { machineId, runId, fromOffset });
}
