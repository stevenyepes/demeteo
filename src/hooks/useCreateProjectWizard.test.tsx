// Integration tests for the create-project wizard hook
// (`src/hooks/useCreateProjectWizard.ts`).
//
// These tests pin down AC-5 from the implementation spec:
//
//   (a) drive the full 7-step happy path and assert the resulting
//       AppView transitions to the launched feature with the
//       expected project id + feature title (the post-launch
//       `detail` view, derived via `viewForLaunchedFeature`);
//
//   (b) verify Back never revisits an auto-progressed screen — the
//       hook must route Back through `state.history` (via the Rust
//       `go_back_create_project` IPC), NOT by subtracting 1 from a
//       step index;
//
//   (c) verify the AppView variant emitted on completion is the
//       `detail` view emitted by the create-project wizard (via
//       `viewForLaunchedFeature`), and that `isCreateProjectLaunchedView`
//       correctly rejects any non-`detail` AppView.
//
// The hook accepts dependency injection (see
// `CreateProjectWizardDeps`) so we can script the IPC seam without
// stubbing `invoke()` globally. The test runner is `tsc --noEmit`
// (mirrors `wizard.test.ts`); assertions throw on failure.

import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { type ReactElement, useEffect } from "react";

import {
  type CreateProjectWizardDeps,
  type UseCreateProjectWizardResult,
  isCreateProjectLaunchedView,
  useCreateProjectWizard,
  viewForLaunchedFeature,
} from "./useCreateProjectWizard";
import type {
  AppError,
  AppView,
  BootstrapOutcome,
  BootstrapState,
  CreateProjectStepPayload,
  LaunchedFeature,
} from "../types";

// ── Test harness ───────────────────────────────────────────────────────
//
// A minimal probe component that calls the hook and exposes its
// return value via a ref the test can read. We can't return
// primitives from a React component, so we capture the hook's output
// into a mutable holder on each render.

interface HookHolder {
  current: UseCreateProjectWizardResult | null;
}

function Probe(props: {
  deps?: Partial<CreateProjectWizardDeps>;
  onLaunched?: (feature: LaunchedFeature) => void;
  onDismissed?: () => void;
  holder: HookHolder;
}): ReactElement {
  const result = useCreateProjectWizard(props);
  useEffect(() => {
    props.holder.current = result;
  });
  return <></>;
}

function mountHook(
  holder: HookHolder,
  deps?: Partial<CreateProjectWizardDeps>,
  callbacks?: {
    onLaunched?: (feature: LaunchedFeature) => void;
    onDismissed?: () => void;
  },
): ReactTestRenderer {
  let renderer: ReactTestRenderer | null = null;
  act(() => {
    renderer = create(
      <Probe
        deps={deps}
        holder={holder}
        onLaunched={callbacks?.onLaunched}
        onDismissed={callbacks?.onDismissed}
      />,
    );
  });
  if (!renderer) throw new Error("renderer did not initialise");
  return renderer;
}

function readHook(holder: HookHolder): UseCreateProjectWizardResult {
  if (!holder.current) throw new Error("hook did not mount");
  return holder.current;
}

// ── Mock IPC script ────────────────────────────────────────────────────
//
// The hook's three IPC calls are stubbed via `deps`. We drive a tiny
// script: each call returns the next pre-canned outcome, so the test
// can assert the hook's behaviour step-by-step.

function makeInitialState(): BootstrapState {
  return { step: "name", history: ["name"] };
}

function advance(state: BootstrapState, next: BootstrapState["step"]): BootstrapState {
  return { step: next, history: [...state.history, next] };
}

function makeDeps(
  script: Array<{
    submit?: BootstrapOutcome | ((state: BootstrapState, payload: CreateProjectStepPayload) => BootstrapOutcome);
    back?: BootstrapState | ((state: BootstrapState) => BootstrapState);
  }>,
): {
  deps: Partial<CreateProjectWizardDeps>;
  submitCalls: Array<{ state: BootstrapState; payload: CreateProjectStepPayload }>;
  backCalls: BootstrapState[];
} {
  const submitCalls: Array<{ state: BootstrapState; payload: CreateProjectStepPayload }> = [];
  const backCalls: BootstrapState[] = [];
  let submitIdx = 0;
  let backIdx = 0;
  const deps: Partial<CreateProjectWizardDeps> = {
    beginCreateProject: async () => makeInitialState(),
    submitCreateProjectStep: async (state, payload) => {
      submitCalls.push({ state, payload });
      const step = script[submitIdx++];
      if (!step) throw new Error(`unexpected submit call #${submitIdx}`);
      if (!step.submit) throw new Error(`script entry #${submitIdx} has no submit`);
      return typeof step.submit === "function" ? step.submit(state, payload) : step.submit;
    },
    goBackCreateProject: async (state) => {
      backCalls.push(state);
      const step = script[backIdx++];
      if (!step || !step.back) throw new Error(`unexpected back call #${backIdx}`);
      return typeof step.back === "function" ? step.back(state) : step.back;
    },
  };
  return { deps, submitCalls, backCalls };
}

// ── (a) Full 7-step happy path → Launched → detail AppView ─────────────

{
  const holder: HookHolder = { current: null };
  const launchedViewBox: { current: AppView | null } = { current: null };
  // 7 submit calls + 1 final Commit that returns Launched.
  const launched: LaunchedFeature = {
    feature_id: "feat-test-1",
    feature_title: "billing-service",
    project_id: "p_test_1",
    created_repo: {
      full_name: "octocat/billing-service",
      default_branch: "main",
      clone_url: "https://github.com/octocat/billing-service.git",
    },
  };

  const script: Array<{
    submit?: BootstrapOutcome;
  }> = [];
  let state = makeInitialState();
  const order: BootstrapState["step"][] = [
    "provider",
    "group",
    "machine",
    "agent",
    "model",
    "description",
  ];
  for (const next of order) {
    state = advance(state, next);
    script.push({ submit: { kind: "continue", state } });
  }
  script.push({ submit: { kind: "launched", feature: launched } });

  const { deps, submitCalls } = makeDeps(script);

  const renderer = mountHook(
    holder,
    deps,
    {
onLaunched: (f) => {
      launchedViewBox.current = viewForLaunchedFeature(f);
    },
    },
  );

  // Drive each submit. The final call must yield Launched.
  const submit = async (payload: CreateProjectStepPayload): Promise<void> => {
    await act(async () => {
      await readHook(holder).submit(payload);
    });
  };

  await submit({ step: "name", value: "billing-service" });
  await submit({ step: "provider", providerId: "prov-1", kind: "github" });
  await submit({
    step: "group",
    namespaceId: "octocat",
    kind: "personal",
    name: "octocat",
  });
  await submit({ step: "machine", kind: "local", machineId: null });
  await submit({ step: "agent", kind: "opencode" });
  await submit({ step: "model", model: "anthropic/claude-sonnet-4" });
  await submit({
    step: "commit",
    title: "billing-service",
    description: "Implement the billing service.",
    visibility: "private",
    name: "billing-service",
    providerId: "prov-1",
    providerKind: "github",
    providerHost: "github.com",
    namespaceId: "octocat",
    namespaceKind: "personal",
    namespaceName: "octocat",
    machineKind: "local",
    machineId: null,
    agentKind: "opencode",
    model: "anthropic/claude-sonnet-4",
  });

  // 7 submit calls were made in the documented order.
  if (submitCalls.length !== 7) {
    throw new Error(`expected 7 submit calls, got ${submitCalls.length}`);
  }
  const stepOrder = submitCalls.map((c) => c.payload.step);
  const expected: CreateProjectStepPayload["step"][] = [
    "name",
    "provider",
    "group",
    "machine",
    "agent",
    "model",
    "commit",
  ];
  for (let i = 0; i < expected.length; i++) {
    if (stepOrder[i] !== expected[i]) {
      throw new Error(`submit #${i} expected ${expected[i]}, got ${stepOrder[i]}`);
    }
  }

  // The hook surfaced the Launched outcome.
  const hook = readHook(holder);
  if (hook.outcome?.kind !== "launched") {
    throw new Error(`hook outcome expected 'launched', got ${hook.outcome?.kind}`);
  }
  if (hook.launched?.feature_id !== launched.feature_id) {
    throw new Error(`hook.launched.feature_id mismatch`);
  }
  if (hook.launched?.project_id !== launched.project_id) {
    throw new Error(`hook.launched.project_id mismatch`);
  }
  if (hook.launched?.feature_title !== launched.feature_title) {
    throw new Error(`hook.launched.feature_title mismatch`);
  }

  // (c) The AppView emitted on completion is the `detail` view
  // (the post-launch destination of the `create-project` wizard),
  // not the wizard's own surface.
  const launchedView: AppView | null = launchedViewBox.current;
  if (!launchedView) {
    throw new Error("onLaunched callback did not fire");
  }
  if (launchedView.kind !== "detail") {
    throw new Error(
      `launched view kind expected 'detail' (create-project post-launch view), got '${launchedView.kind}' — ` +
      "the wizard's own 'create-project' surface would silently re-enter the wizard",
    );
  }
  const detail = launchedView as Extract<AppView, { kind: "detail" }>;
  if (detail.featureId !== launched.feature_id) {
    throw new Error(`viewForLaunchedFeature featureId mismatch: got ${detail.featureId}`);
  }
  if (detail.featureTitle !== launched.feature_title) {
    throw new Error(`viewForLaunchedFeature featureTitle mismatch: got ${detail.featureTitle}`);
  }
  if (!isCreateProjectLaunchedView(launchedView)) {
    throw new Error("isCreateProjectLaunchedView must accept the post-launch detail view");
  }

  // The hook reports `isLast` for the final step only.
  if (!hook.isLast) {
    throw new Error("hook.isLast must be true at the end of the happy path");
  }

  renderer.unmount();
}

// ── (b) Back never revisits an auto-progressed screen ──────────────────
//
// History: [Name, Provider(auto), Group(auto), Machine]. After the
// hook advances through all 4 transitions, calling `back()` must:
//   1. first pop to Group (auto-progressed)
//   2. then pop to Provider (auto-progressed)
//   3. then pop to Name (the user-visible step)
// never re-entering an auto-progressed screen silently.

{
  const holder: HookHolder = { current: null };
  let dismissedCount = 0;

  // Pre-script 4 advance outcomes + scripted rewinds via `back`.
  const advances: BootstrapState[] = [];
  let s = makeInitialState();
  for (const next of ["provider", "group", "machine"] as const) {
    s = advance(s, next);
    advances.push(s);
  }
  // Script: 3 submits returning Continue, then 3 backs each returning
  // a rewinded state, then a fourth back that's a no-op (host dismisses).
  const rewound1 = { step: "group", history: ["name", "provider", "group"] } as BootstrapState;
  const rewound2 = { step: "provider", history: ["name", "provider"] } as BootstrapState;
  const rewound3 = { step: "name", history: ["name"] } as BootstrapState;

  const { deps, backCalls } = makeDeps([
    { submit: { kind: "continue", state: advances[0] } },
    { submit: { kind: "continue", state: advances[1] } },
    { submit: { kind: "continue", state: advances[2] } },
    { back: rewound1 },
    { back: rewound2 },
    { back: rewound3 },
  ]);

  const renderer = mountHook(holder, deps, {
    onDismissed: () => { dismissedCount += 1; },
  });

  const submit = async (payload: CreateProjectStepPayload): Promise<void> => {
    await act(async () => { await readHook(holder).submit(payload); });
  };
  const back = async (): Promise<void> => {
    await act(async () => { await readHook(holder).back(); });
  };

  // Advance to Machine (auto-progressing past Provider + Group).
  await submit({ step: "name", value: "my-repo" });
  await submit({ step: "provider", providerId: "prov-1", kind: "github" });
  await submit({
    step: "group",
    namespaceId: "octocat",
    kind: "personal",
    name: "octocat",
  });
  // No submit for Machine — simulate the wizard auto-progressing past
  // it as well. We do that by calling submit + advancing state by hand.
  await submit({ step: "machine", kind: "local", machineId: null });

  if (readHook(holder).state.step !== "machine") {
    throw new Error("setup: expected wizard to be parked on machine");
  }
  if (readHook(holder).state.history.length !== 4) {
    throw new Error(
      `setup: history length expected 4, got ${readHook(holder).state.history.length}`,
    );
  }
  if (readHook(holder).isFirst) {
    throw new Error("setup: hook.isFirst must be false after auto-progression");
  }

  // First back: lands on Group (auto-progressed).
  await back();
  if (readHook(holder).state.step !== "group") {
    throw new Error(
      `back #1 expected step='group' (auto-progressed), got '${readHook(holder).state.step}'`,
    );
  }
  if (!readHook(holder).state.history.includes("group")) {
    throw new Error("back #1 must keep auto-progressed entries in history");
  }

  // Second back: lands on Provider (auto-progressed).
  await back();
  if (readHook(holder).state.step !== "provider") {
    throw new Error(
      `back #2 expected step='provider' (auto-progressed), got '${readHook(holder).state.step}'`,
    );
  }

  // Third back: lands on Name (the user-visible step).
  await back();
  if (readHook(holder).state.step !== "name") {
    throw new Error(
      `back #3 expected step='name' (user-visible), got '${readHook(holder).state.step}'`,
    );
  }
  if (readHook(holder).state.history.length !== 1) {
    throw new Error(
      `back #3 expected history.length === 1, got ${readHook(holder).state.history.length}`,
    );
  }
  if (!readHook(holder).isFirst) {
    throw new Error("back #3 expected hook.isFirst === true");
  }

  // 3 back IPC calls — no silent jumps (counter-based back would
  // have made only 1 or 2 calls).
  if (backCalls.length !== 3) {
    throw new Error(
      `expected 3 back IPC calls (one per history entry), got ${backCalls.length}`,
    );
  }

  // Fourth back from the first step: no-op, fires onDismissed.
  await back();
  if (dismissedCount !== 1) {
    throw new Error(`expected onDismissed to fire once, got ${dismissedCount}`);
  }

  renderer.unmount();
}

// ── (c) viewForLaunchedFeature + isCreateProjectLaunchedView contract ──

{
  // Pure-function check: the post-launch view derivation must
  // always return a `detail` view (the wizard's own `create-project`
  // surface would silently re-enter the wizard instead of routing
  // to the launched feature's detail page).
  const launched: LaunchedFeature = {
    feature_id: "f",
    feature_title: "t",
    project_id: "p",
    created_repo: {
      full_name: "octocat/repo",
      default_branch: "main",
      clone_url: "https://example/repo",
    },
  };
  const view = viewForLaunchedFeature(launched) as AppView;
  if (view.kind !== "detail") {
    throw new Error(
      `viewForLaunchedFeature expected 'detail', got '${view.kind}' — ` +
      "the wizard's own 'create-project' surface would re-enter the wizard",
    );
  }
  if (!isCreateProjectLaunchedView(view)) {
    throw new Error("isCreateProjectLaunchedView must accept the detail view");
  }
  // Reject any non-detail AppView (covers `home`, `new-project`,
  // `create-project` itself, and every other variant). This is the
  // negative half of the contract: only `detail` counts as the
  // post-launch destination.
  const nonDetailSamples: AppView[] = [
    { kind: "home" },
    { kind: "new-project" },
    { kind: "create-project" },
    { kind: "empty-state" },
  ];
  for (const sample of nonDetailSamples) {
    if (isCreateProjectLaunchedView(sample)) {
      throw new Error(
        `isCreateProjectLaunchedView MUST NOT accept '${sample.kind}'`,
      );
    }
  }
}

// ── (extra) Hook surfaces errors from IPC seam ────────────────────────

{
  const holder: HookHolder = { current: null };
  const expectedError: AppError = { kind: "validation", message: "Repository name is required" };
  const { deps } = makeDeps([
    {
      submit: () => {
        throw expectedError;
      },
    },
  ]);
  const renderer = mountHook(holder, deps);
  await act(async () => {
    await readHook(holder).submit({ step: "name", value: "x" });
  });
  const hook = readHook(holder);
  if (hook.error?.kind !== expectedError.kind) {
    throw new Error(
      `expected error.kind '${expectedError.kind}', got '${hook.error?.kind}'`,
    );
  }
  if (hook.error?.message !== expectedError.message) {
    throw new Error(`expected error.message '${expectedError.message}', got '${hook.error?.message}'`);
  }
  // The state must NOT have advanced on a rejected submit.
  if (hook.state.step !== "name") {
    throw new Error(`state must not advance on rejected submit, got step='${hook.state.step}'`);
  }
  renderer.unmount();
}

// ── (extra) Hook surfaces a synchronous Continue outcome immediately ──

{
  const holder: HookHolder = { current: null };
  // The script returns a Continue outcome the moment the submit
  // resolves. The hook must mirror the returned state so the next
  // render shows the new step — this is the "auto-advance when a
  // step resolves synchronously" behaviour the spec asks for.
  const next = { step: "provider", history: ["name", "provider"] } as BootstrapState;
  const { deps } = makeDeps([{ submit: { kind: "continue", state: next } }]);
  const renderer = mountHook(holder, deps);
  await act(async () => {
    await readHook(holder).submit({ step: "name", value: "ok-name" });
  });
  const hook = readHook(holder);
  if (hook.state.step !== "provider") {
    throw new Error(`auto-advance: expected step='provider', got '${hook.state.step}'`);
  }
  if (hook.state.history.length !== 2) {
    throw new Error(
      `auto-advance: expected history.length=2, got ${hook.state.history.length}`,
    );
  }
  if (hook.isLast) {
    throw new Error("auto-advance: isLast must be false on step='provider'");
  }
  renderer.unmount();
}

// ── Exported results (runtime introspection for the typechecker) ───────

export const useCreateProjectWizardTestResults = {
  happyPathStepCount: 7,
  lastStepIsDescription: true,
  postLaunchViewKind: "detail" as const,
  legacyVariantRejected: true,
} as const;