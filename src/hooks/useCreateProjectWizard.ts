import { useCallback, useEffect, useRef, useState } from "react";
import {
  beginCreateProject as defaultBeginCreateProject,
  goBackCreateProject as defaultGoBackCreateProject,
  isCreateProjectLaunchedView,
  isFirstStep,
  isLastStep,
  submitCreateProjectStep as defaultSubmitCreateProjectStep,
  viewForLaunchedFeature,
  wizardError,
} from "../lib/createProject";
import type {
  AppError,
  BootstrapOutcome,
  BootstrapState,
  CreateProjectStepPayload,
  LaunchedFeature,
} from "../types";

// ── Dependency injection surface ────────────────────────────────────────
//
// The hook's three IPC calls are exposed as injectable seams so unit
// tests can drive the hook end-to-end without stubbing `invoke()`
// globally. Production callers leave the defaults in place; tests
// pass mocks that return scripted `BootstrapOutcome` values.

/** Contract the hook depends on. Default impls come from
 *  `src/lib/createProject.ts` and talk to the Rust commands. */
export interface CreateProjectWizardDeps {
  beginCreateProject: () => Promise<BootstrapState>;
  submitCreateProjectStep: (
    state: BootstrapState,
    payload: CreateProjectStepPayload,
  ) => Promise<BootstrapOutcome>;
  goBackCreateProject: (state: BootstrapState) => Promise<BootstrapState>;
}

const DEFAULT_DEPS: CreateProjectWizardDeps = {
  beginCreateProject: defaultBeginCreateProject,
  submitCreateProjectStep: defaultSubmitCreateProjectStep,
  goBackCreateProject: defaultGoBackCreateProject,
};

// ── Hook return shape ───────────────────────────────────────────────────
//
// Public surface: `{ state, submit, back, isFirst, isLast }`. The
// `outcome` + `launched` fields are exposed as read-only so unit
// tests can assert them without re-running the IPC. Production
// callers ignore them.

export interface UseCreateProjectWizardResult {
  /** The current `BootstrapState`. Mirrors the Rust state machine
   *  1-to-1 (including the `history` log). */
  state: BootstrapState;
  /** Submit the current step's payload. On `BootstrapOutcome::Continue`
   *  the state is updated in place (auto-advance). On
   *  `BootstrapOutcome::Launched` the wizard halts — the launched
   *  feature is surfaced via `launched` and the view reported via
   *  `onLaunched` / the `outcome` field. */
  submit: (payload: CreateProjectStepPayload) => Promise<void>;
  /** Rewind the wizard by one step. Calls the Rust `go_back_create_project`
   *  which pops the current step off `history` — auto-progressed
   *  entries are still rewindable. */
  back: () => Promise<void>;
  /** True iff the wizard is on its first step. Drives the Back button's
   *  disabled state. */
  isFirst: boolean;
  /** True iff the wizard is on its final `Description` step. Drives the
   *  Next-button label and the `commit` payload shape. */
  isLast: boolean;
  /** Last outcome returned by `submit`. Cleared on a new submit. */
  outcome: BootstrapOutcome | null;
  /** Convenience accessor for `outcome.kind === 'launched'`. */
  launched: LaunchedFeature | null;
  /** Last error captured from any IPC. Cleared on a new submit/back. */
  error: AppError | null;
  /** True iff the hook has finished its initial `begin_create_project`
   *  call. Mirrors the Rust `BootstrapState::new()` shape so the UI
   *  can render something deterministic before the IPC resolves. */
  ready: boolean;
}

export interface UseCreateProjectWizardOptions {
  /** Override the IPC seams (testing only). When omitted the hook uses
   *  the production wrappers from `src/lib/createProject.ts`. */
  deps?: Partial<CreateProjectWizardDeps>;
  /** Optional callback invoked once when the wizard completes. Receives
   *  the launched feature. Production callers wire this to
   *  `navigate({ kind: 'detail', featureId, featureTitle })`. */
  onLaunched?: (feature: LaunchedFeature) => void;
  /** Optional callback invoked once when the wizard bails out (back on
   *  first step). Lets the host dismiss the wizard cleanly. */
  onDismissed?: () => void;
}

// ── The hook ────────────────────────────────────────────────────────────

/** Owns the create-from-zero wizard's `BootstrapState` and exposes a
 *  narrow, testable surface (`{ state, submit, back, isFirst, isLast }`).
 *
 *  **State-machine authority.** The hook holds the canonical
 *  `BootstrapState` so the React tree never has to think about IPC
 *  plumbing. Every state mutation goes through `submit` / `back`,
 *  both of which call the Rust side and then mirror the returned
 *  state. Auto-advance is implicit: when `submit` returns a
 *  `BootstrapOutcome::Continue`, the returned state's `step` already
 *  points at the next screen and React re-renders accordingly.
 *
 *  **Back routing.** `back()` always calls `goBackCreateProject` —
 *  the Rust command pops `state.history` so an auto-progressed entry
 *  is still rewound. The hook never subtracts 1 from an index into
 *  `STEP_ORDER`; doing so would silently re-enter auto-progressed
 *  screens (C-4 regression).
 *
 *  **Auto-progressed screens.** The wizard can advance past a step
 *  without a user decision (e.g. only one provider configured →
 *  skip the Provider screen). Such transitions are recorded in
 *  `state.history` and `back()` rewinds through them in order. The
 *  `back()` call always routes through the Rust `go_back_create_project`
 *  IPC so the history-pop logic lives in exactly one place.
 *
 *  @example
 *  ```tsx
 *  const { state, submit, back, isFirst, isLast } = useCreateProjectWizard({
 *    onLaunched: (f) => navigate({
 *      kind: 'detail', featureId: f.feature_id, featureTitle: f.feature_title,
 *    }),
 *    onDismissed: () => navigate({ kind: 'home' }),
 *  });
 *  ```
 */
export function useCreateProjectWizard(
  options: UseCreateProjectWizardOptions = {},
): UseCreateProjectWizardResult {
  const deps: CreateProjectWizardDeps = { ...DEFAULT_DEPS, ...(options.deps ?? {}) };
  const onLaunchedRef = useRef(options.onLaunched);
  const onDismissedRef = useRef(options.onDismissed);
  onLaunchedRef.current = options.onLaunched;
  onDismissedRef.current = options.onDismissed;

  // Seed the state with the deterministic Rust `BootstrapState::new()`
  // shape so the UI renders something consistent before the IPC
  // resolves. Mirrors the React wizard's existing seed.
  const [state, setState] = useState<BootstrapState>(() => ({
    step: "name",
    history: ["name"],
  }));
  const [outcome, setOutcome] = useState<BootstrapOutcome | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [ready, setReady] = useState(false);

  // ── Begin a wizard session on mount ──────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const initial = await deps.beginCreateProject();
        if (cancelled) return;
        setState(initial);
        setReady(true);
      } catch (err) {
        if (cancelled) return;
        setError(wizardError(err) ?? { kind: "internal", message: String(err) });
      }
    })();
    return () => { cancelled = true; };
    // We deliberately only re-run if the begin-call changes identity.
    // In production `deps.beginCreateProject` is a stable reference
    // from `createProject.ts`; tests pass fresh mocks per render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deps.beginCreateProject]);

  // ── Submit ──────────────────────────────────────────────────────────
  //
  // Drives the state machine forward. Two outcomes:
  //   - Continue: replace the state with the returned one
  //     (auto-advance happens for free — the new state's `step` is
  //     what React re-renders).
  //   - Launched: stop the wizard, surface the launched feature to
  //     the host via `onLaunched`. The hook clears `error` so any
  //     prior inline error goes away on the next attempt.
  const submit = useCallback(
    async (payload: CreateProjectStepPayload): Promise<void> => {
      setError(null);
      try {
        const next = await deps.submitCreateProjectStep(state, payload);
        setOutcome(next);
        if (next.kind === "continue") {
          setState(next.state);
        } else {
          onLaunchedRef.current?.(next.feature);
        }
      } catch (err) {
        setError(wizardError(err) ?? { kind: "internal", message: String(err) });
      }
    },
    [deps, state],
  );

  // ── Back ────────────────────────────────────────────────────────────
  //
  // Routes through the Rust `go_back_create_project` IPC which pops
  // `state.history`. The hook NEVER subtracts 1 from an index into
  // `STEP_ORDER` — that's the auto-progressed-screen bug this hook
  // is designed to avoid (C-4 regression). When the wizard is on its
  // first step the Rust command is a no-op, and the host's
  // `onDismissed` callback is invoked so the wizard can be dismissed
  // cleanly.
  const back = useCallback(async (): Promise<void> => {
    setError(null);
    if (!canRewindLocal(state)) {
      onDismissedRef.current?.();
      return;
    }
    try {
      const rewound = await deps.goBackCreateProject(state);
      setState(rewound);
      // Going back always implies staying in the wizard, so any
      // prior outcome is cleared (the user could re-trigger submit
      // and we'd otherwise surface a stale `Launched`).
      setOutcome(null);
    } catch (err) {
      setError(wizardError(err) ?? { kind: "internal", message: String(err) });
    }
  }, [deps, state]);

  const launched = outcome?.kind === "launched" ? outcome.feature : null;

  return {
    state,
    submit,
    back,
    isFirst: isFirstStep(state),
    isLast: isLastStep(state),
    outcome,
    launched,
    error,
    ready,
  };
}

// ── Local helpers ──────────────────────────────────────────────────────

function canRewindLocal(state: BootstrapState): boolean {
  return state.history.length > 1;
}

// ── Re-exports ─────────────────────────────────────────────────────────
//
// Surface the post-launch view derivation so callers don't have to
// reach into `src/lib/createProject.ts` themselves. Both helpers are
// pure and the hook test imports them directly.

export { viewForLaunchedFeature, isCreateProjectLaunchedView };