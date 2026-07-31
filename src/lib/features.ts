import { invoke } from "@tauri-apps/api/core";
import { asAppError } from "./errors";
import type {
  EffortLevel,
  Feature,
  SequenceState,
  StepAttempt,
  StepExecution,
} from "../types";

/** Features still executing (or awaiting a gate) in a project. */
export async function fetchActiveFeatures(projectId: string): Promise<Feature[]> {
  return invoke<Feature[]>("fetch_active_features", { projectId });
}

/** Where a feature's worktree lives, resolved at call time. Mirrors the Rust
 *  `FeatureWorktreeInfo`, which the runner's `get_worktree` RPC returns too —
 *  so a caller opening a terminal on it needs no transport branch. */
export interface FeatureWorktreeInfo {
  machine_id: string;
  worktree_path: string;
  branch: string;
  default_branch: string;
}

export async function getFeatureWorktree(featureId: string): Promise<FeatureWorktreeInfo> {
  return invoke<FeatureWorktreeInfo>("feature_get_worktree", { featureId });
}

export async function getStepExecution(executionId: string): Promise<StepExecution> {
  return invoke<StepExecution>("step_get", { executionId });
}

export async function listStepsForRun(featureId: string): Promise<StepExecution[]> {
  return invoke<StepExecution[]>("step_list_for_run", { featureId });
}

/** Per-attempt history for one step execution. */
export async function listStepAttempts(executionId: string): Promise<StepAttempt[]> {
  return invoke<StepAttempt[]>("step_attempts_list", { executionId });
}

/** A `sequence` node's task list, split into the landed prefix and what is
 *  still pending. `nodeId` is the graph node id; a non-sequence or
 *  not-yet-planned node reads back `unplanned`. */
export async function getSequenceState(input: {
  featureId: string;
  nodeId: string;
  executionId: string;
}): Promise<SequenceState> {
  return invoke<SequenceState>("sequence_tasks_list", {
    featureId: input.featureId,
    nodeId: input.nodeId,
    executionId: input.executionId,
  });
}

/**
 * The UTF-8 body of a run's declared artifact. A *display* read of a run
 * surface, so it goes through `RunView` rather than `sftp_read_file` — the
 * seam that lets a runner-owned feature's artifact resolve from the laptop
 * shadow. General filesystem browsing stays on `lib/files.ts`.
 */
export async function artifactBody(machineId: string, path: string): Promise<string> {
  return invoke<string>("artifact_body", { machineId, path });
}

/**
 * Subset of `StepExecution` the gate-modal block-banner actually needs.
 * Kept narrow so the type can be reused by callers that only fetch the
 * predecessor set (without dragging the whole row in).
 */
export type GateBlocker = Pick<StepExecution, "id" | "step_id" | "status" | "step_index">;

/**
 * Phrases the backend uses to describe a blocked retry / gate decision.
 * Kept in lock-step with the `assert_no_active_predecessors` helper in
 * `src-tauri/src/adapters/step_executor/impl_traits/mod.rs`. Used by
 * `isBlockingError` to detect the precondition violation regardless of
 * whether the error surface carries `AppError::validation` (preferred)
 * or the legacy string-only path.
 */
export const BLOCKING_ERROR_PHRASES = [
  "is still pending",
  "is still running",
  "is still verifying",
  "is still awaiting",
] as const;

/**
 * Decide a manual gate step (`approve | redirect | cancel`). Wraps
 * the raw `invoke('gate_decide', …)` call so the UI has one
 * centralised place to detect the blocking-predecessor error and
 * surface it with a tailored toast.
 */
export async function decideGate(input: {
  stepExecutionId: string;
  decision: "approve" | "redirect" | "cancel";
  feedback: string | null;
}): Promise<void> {
  await invoke<void>("gate_decide", {
    stepExecutionId: input.stepExecutionId,
    decision: input.decision,
    feedback: input.feedback,
  });
}

/**
 * Retry a failed / interrupted / pending step. Re-pins the
 * feature-wide model / harness / effort overrides before the rerun
 * (`null` keeps the existing override).
 */
export async function retryStep(input: {
  stepExecutionId: string;
  newModel: string | null;
  newAgent: string | null;
  newEffort: EffortLevel | null;
}): Promise<void> {
  await invoke<void>("step_retry", {
    stepExecutionId: input.stepExecutionId,
    newModel: input.newModel,
    newAgent: input.newAgent,
    newEffort: input.newEffort,
  });
}

/**
 * Rewind to a step and re-execute it plus everything downstream, re-pinning
 * the same three overrides as {@link retryStep}.
 *
 * Close to {@link retryStep} but not the same call, and the differences bite:
 * this one accepts a step of *any* status (a replay target is normally
 * `completed`) and drops a sequence step's landed checkpoint so it runs its
 * whole task list again.
 */
export async function replayFromStep(input: {
  stepExecutionId: string;
  newModel: string | null;
  newAgent: string | null;
  newEffort: EffortLevel | null;
}): Promise<void> {
  await invoke<void>("replay_from_step", {
    stepExecutionId: input.stepExecutionId,
    newModel: input.newModel,
    newAgent: input.newAgent,
    newEffort: input.newEffort,
  });
}

/**
 * The detached twin of {@link retryStep}: a run the runner owns is rewound
 * *on the runner* (this machine has neither its driver nor its worktree).
 *
 * Pair it with {@link remoteReplayStep} rather than reusing it for both.
 * Routing replay through here is exactly what broke remote replay: the
 * runner's retry arm calls `step_retry`, which rejects any step that is not
 * `failed` / `interrupted` / `pending` — so replaying from a completed step
 * failed with "Cannot retry a step in 'completed' status".
 */
export async function remoteRetryStep(input: {
  machineId: string;
  runId: string;
  stepExecutionId: string;
  model: string | null;
  agentKind: string | null;
  effort: EffortLevel | null;
}): Promise<void> {
  await invoke<void>("remote_retry_step", {
    machineId: input.machineId,
    runId: input.runId,
    stepExecutionId: input.stepExecutionId,
    model: input.model,
    agentKind: input.agentKind,
    effort: input.effort,
  });
}

/** The detached twin of {@link replayFromStep} — see {@link remoteRetryStep}. */
export async function remoteReplayStep(input: {
  machineId: string;
  runId: string;
  stepExecutionId: string;
  model: string | null;
  agentKind: string | null;
  effort: EffortLevel | null;
}): Promise<void> {
  await invoke<void>("remote_replay_step", {
    machineId: input.machineId,
    runId: input.runId,
    stepExecutionId: input.stepExecutionId,
    model: input.model,
    agentKind: input.agentKind,
    effort: input.effort,
  });
}

/**
 * Returns `true` when the rejected promise represents a
 * blocking-predecessor error from `step_retry` / `gate_decide`.
 *
 * The backend constructs these as `AppError::validation` with one of
 * the phrases in {@link BLOCKING_ERROR_PHRASES}. Callers should route
 * blocking errors to a `warning` toast instead of an `error` toast,
 * since the user did the right thing (the UI was stale).
 */
export function isBlockingError(err: unknown): boolean {
  const appErr = asAppError(err);
  if (appErr?.kind !== "validation") return false;
  return BLOCKING_ERROR_PHRASES.some((phrase) => appErr.message.includes(phrase));
}

/**
 * Prefix the backend puts on every terminal "the machine is not provisioned"
 * failure (`build_environment_message`). Kept as a constant so the UI's
 * environment hint and the message stay wired to the same string.
 */
export const ENVIRONMENT_ERROR_PREFIX = "Environment not ready";

/**
 * Returns `true` when a step's `error_message` is the backend's terminal
 * environment failure rather than a code failure — i.e. the step did not fail
 * because the change is wrong, it failed because the machine could not run the
 * command at all. The UI uses this to swap "retry the step" framing for "fix
 * the machine" framing, since retrying an unprovisioned box just fails again.
 */
export function isEnvironmentError(errorMessage: string | null | undefined): boolean {
  return (errorMessage ?? "").trimStart().startsWith(ENVIRONMENT_ERROR_PREFIX);
}

/**
 * Pure helper: find the first non-terminal predecessor of `target`
 * in `steps`, ordered by `step_index`. Returns `null` when no
 * predecessor is blocking — used by the UI to decide whether to
 * disable Retry / Approve buttons before the IPC round-trip.
 *
 * The backend enforces the same rule via
 * `assert_no_active_predecessors`; this is the defence-in-depth layer
 * that makes the buttons feel right to the user without a round-trip.
 */
export function findActivePredecessor(
  steps: readonly StepExecution[],
  target: Pick<StepExecution, "id" | "step_index">,
): StepExecution | null {
  for (const s of steps) {
    if (s.id === target.id) continue;
    if (s.step_index >= target.step_index) continue;
    if (
      s.status === "pending" ||
      s.status === "running" ||
      s.status === "verifying" ||
      s.status === "awaiting_gate"
    ) {
      return s;
    }
  }
  return null;
}

/**
 * Same logic as {@link findActivePredecessor} but narrowed to the
 * fields needed to render the gate-modal blocking banner. Pulls the
 * full step list via `step_list_for_run` so the caller doesn't need
 * to thread the steps through the modal props.
 */
export async function listBlockingPredecessor(
  featureId: string,
  target: Pick<StepExecution, "id" | "step_index">,
): Promise<GateBlocker | null> {
  const steps = await listStepsForRun(featureId);
  const found = findActivePredecessor(steps, target);
  if (!found) return null;
  return {
    id: found.id,
    step_id: found.step_id,
    status: found.status,
    step_index: found.step_index,
  };
}