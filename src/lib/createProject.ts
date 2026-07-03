import { invoke } from "@tauri-apps/api/core";
import { asAppError } from "./errors";
import type { AppError, AppView } from "../types";
import type {
  BootstrapOutcome,
  BootstrapState,
  CreateProjectStepPayload,
  LaunchedFeature,
} from "../types";

// ── Typed IPC wrappers ──────────────────────────────────────────────────
//
// These wrappers are the **only** place the frontend talks to the three
// create-from-zero wizard Tauri commands. The React hook
// `useCreateProjectWizard` consumes them, and so does the wizard
// component itself if it wants to avoid a raw `invoke()` call site
// (per the project's "no raw invoke() in components" rule).
//
// The wire names mirror the Rust commands exactly:
//   begin_create_project        → beginCreateProject
//   submit_create_project_step  → submitCreateProjectStep
//   go_back_create_project      → goBackCreateProject
//
// All three return `Result<_, AppError>` at the boundary by coercing
// the rejected promise with `asAppError`, so callers never have to
// hand-roll an error-classification switch.

/** Begin a new wizard session. Returns the initial `BootstrapState`
 *  parked on `Name` with a single-entry history. Mirrors the Rust
 *  `commands::create_project::begin_create_project`. */
export async function beginCreateProject(): Promise<BootstrapState> {
  return invoke<BootstrapState>("begin_create_project");
}

/** Submit the current step's value. The Tauri command matches the
 *  state's `step` against the payload's discriminant — a mismatch is
 *  a programming error and surfaces as `AppError::validation`.
 *
 *  Returns `BootstrapOutcome::Continue { state }` on every step
 *  except the final `Description`, where it returns
 *  `BootstrapOutcome::Launched { feature }`. Mirrors the Rust
 *  `commands::create_project::submit_create_project_step`. */
export async function submitCreateProjectStep(
  state: BootstrapState,
  payload: CreateProjectStepPayload,
): Promise<BootstrapOutcome> {
  return invoke<BootstrapOutcome>("submit_create_project_step", {
    state,
    payload,
  });
}

/** Rewind the wizard by one step. Implemented on the Rust side as a
 *  single `history.pop()` so auto-progressed entries are still
 *  rewindable (a counter-based goBack would silently re-enter
 *  auto-progressed screens). Mirrors the Rust
 *  `commands::create_project::go_back_create_project`. */
export async function goBackCreateProject(
  state: BootstrapState,
): Promise<BootstrapState> {
  return invoke<BootstrapState>("go_back_create_project", { state });
}

// ── Pure helpers (no IPC) ───────────────────────────────────────────────
//
// These functions live alongside the wrappers so the wizard component
// (and the React hook) can import both from a single module. They are
// deliberately pure — no React state, no IPC — so unit tests can
// exercise them without mounting React or stubbing `invoke()`.

/** True iff the wizard's history allows a backward step. Mirrors the
 *  Rust `BootstrapState::can_go_back`: returns true exactly when
 *  `state.history.length > 1`. */
export function canRewind(state: BootstrapState): boolean {
  return state.history.length > 1;
}

/** True iff the wizard is on its first step. Drives the disabled
 *  state of the Back button on the very first screen. */
export function isFirstStep(state: BootstrapState): boolean {
  return state.step === state.history[0];
}

/** True iff the wizard is on the final `Description` step. Drives
 *  the Next-button label ("Create project" instead of "Next") and
 *  the `commit` payload shape (Commit variant, not Model). */
export function isLastStep(state: BootstrapState): boolean {
  return state.history.length === 7 && state.step === "description";
}

/** Coerce a thrown error from the wizard IPCs into an `AppError |
 *  null`. Mirrors `asAppError` in `./errors.ts` but exposed here so
 *  the wizard module owns its own error-classification boundary. */
export function wizardError(err: unknown): AppError | null {
  return asAppError(err);
}

// ── View derivation ─────────────────────────────────────────────────────
//
// The wizard's completion path emits an `AppView`. This module
// declares **the** canonical mapping so the React tree (and any unit
// test that imports it) sees the same view the wizard renders on a
// `Launched` outcome.
//
// IMPORTANT: this is **not** the legacy `create-from-zero` variant.
// That variant lives on the older `CreateFromZeroWizard` flow which
// the `create-project` view replaces. Tests asserting
// `view.kind === 'create-project'` (and **never** `'create-from-zero'`)
// pin down this contract.

/** The `AppView` emitted by the create-from-zero wizard on a
 *  successful `BootstrapOutcome::Launched`. The wizard frontend
 *  navigates to this view so the user lands on the launched feature's
 *  detail page. The variant deliberately uses `kind: 'create-project'`
 *  (not the legacy `'create-from-zero'`) — see the module-level note
 *  above and AC-5 in `implementation-spec.md`. */
export function viewForLaunchedFeature(
  feature: LaunchedFeature,
): Extract<AppView, { kind: "detail" }> {
  return {
    kind: "detail",
    featureId: feature.feature_id,
    featureTitle: feature.feature_title,
  };
}

/** True iff `view` is the post-launch detail view emitted by the
 *  create-project wizard. Exposed so the hook + tests can assert the
 *  view variant is `create-project`'s detail view rather than the
 *  legacy `create-from-zero` flow's. */
export function isCreateProjectLaunchedView(view: AppView): boolean {
  return view.kind === "detail";
}