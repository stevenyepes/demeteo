import { invoke } from "@tauri-apps/api/core";
import { asAppError } from "./errors";
import type { StepExecution } from "../types";

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
 * feature-wide model / harness overrides before the rerun
 * (`null` keeps the existing override).
 */
export async function retryStep(input: {
  stepExecutionId: string;
  newModel: string | null;
  newAgent: string | null;
}): Promise<void> {
  await invoke<void>("step_retry", {
    stepExecutionId: input.stepExecutionId,
    newModel: input.newModel,
    newAgent: input.newAgent,
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
  const steps = await invoke<StepExecution[]>("step_list_for_run", { featureId });
  const found = findActivePredecessor(steps, target);
  if (!found) return null;
  return {
    id: found.id,
    step_id: found.step_id,
    status: found.status,
    step_index: found.step_index,
  };
}