// Integration tests for the create-project wizard hook
// (`src/hooks/useCreateProjectWizard.ts`).
//
// These pin down AC-5 from the implementation spec:
//
//   (a) drive the full 7-step happy path and assert the resulting AppView
//       transitions to the launched feature's `detail` view (via
//       `viewForLaunchedFeature`);
//
//   (b) verify Back never revisits an auto-progressed screen — the hook must
//       route Back through `state.history` (via the Rust `go_back_create_project`
//       IPC), NOT by subtracting 1 from a step index;
//
//   (c) verify `isCreateProjectLaunchedView` accepts only the post-launch
//       `detail` view.
//
// The hook takes dependency injection (`CreateProjectWizardDeps`) so the IPC
// seam is scripted here rather than by stubbing `invoke()` globally.

import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  type CreateProjectWizardDeps,
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

function makeInitialState(): BootstrapState {
  return { step: "name", history: ["name"] };
}

function advance(state: BootstrapState, next: BootstrapState["step"]): BootstrapState {
  return { step: next, history: [...state.history, next] };
}

type SubmitOutcome =
  | BootstrapOutcome
  | ((state: BootstrapState, payload: CreateProjectStepPayload) => BootstrapOutcome);

// Scripts the hook's IPC seam.
//
// `submits` and `backs` are SEPARATE queues on purpose. An earlier version of
// this harness kept one `script` array and indexed it with two independent
// cursors, so the first `back()` consumed the first *submit* entry and threw.
// The file was never executed by a runner, so the bug went unnoticed.
function makeDeps(script: { submits?: SubmitOutcome[]; backs?: BootstrapState[] }) {
  const submits = [...(script.submits ?? [])];
  const backs = [...(script.backs ?? [])];

  const submitCalls: Array<{ state: BootstrapState; payload: CreateProjectStepPayload }> = [];
  const backCalls: BootstrapState[] = [];

  const deps: Partial<CreateProjectWizardDeps> = {
    beginCreateProject: async () => makeInitialState(),

    submitCreateProjectStep: async (state, payload) => {
      submitCalls.push({ state, payload });
      const outcome = submits.shift();
      if (!outcome) throw new Error(`unscripted submit call #${submitCalls.length}`);
      return typeof outcome === "function" ? outcome(state, payload) : outcome;
    },

    goBackCreateProject: async (state) => {
      backCalls.push(state);
      const rewound = backs.shift();
      if (!rewound) throw new Error(`unscripted back call #${backCalls.length}`);
      return rewound;
    },
  };

  return { deps, submitCalls, backCalls };
}

function mountHook(
  deps: Partial<CreateProjectWizardDeps>,
  callbacks: {
    onLaunched?: (feature: LaunchedFeature) => void;
    onDismissed?: () => void;
  } = {},
) {
  const { result } = renderHook(() => useCreateProjectWizard({ deps, ...callbacks }));

  const submit = async (payload: CreateProjectStepPayload) => {
    await act(async () => {
      await result.current.submit(payload);
    });
  };

  const back = async () => {
    await act(async () => {
      await result.current.back();
    });
  };

  return { result, submit, back };
}

const LAUNCHED: LaunchedFeature = {
  feature_id: "feat-test-1",
  feature_title: "billing-service",
  project_id: "p_test_1",
  created_repo: {
    full_name: "octocat/billing-service",
    default_branch: "main",
    clone_url: "https://github.com/octocat/billing-service.git",
  },
};

describe("(a) the full 7-step happy path", () => {
  it("submits every step in order and lands on the launched feature's detail view", async () => {
    // Six Continue outcomes walking name → description, then the Commit that
    // returns Launched.
    const submits: SubmitOutcome[] = [];
    let state = makeInitialState();
    for (const next of ["provider", "group", "machine", "agent", "model", "description"] as const) {
      state = advance(state, next);
      submits.push({ kind: "continue", state });
    }
    submits.push({ kind: "launched", feature: LAUNCHED });

    const { deps, submitCalls } = makeDeps({ submits });
    let launchedView: AppView | null = null;
    const { result, submit } = mountHook(deps, {
      onLaunched: (f) => {
        launchedView = viewForLaunchedFeature(f);
      },
    });

    await submit({ step: "name", value: "billing-service" });
    await submit({ step: "provider", provider_id: "prov-1", kind: "github" });
    await submit({ step: "group", namespace_id: "octocat", kind: "personal", name: "octocat" });
    await submit({ step: "machine", kind: "local", machine_id: null });
    await submit({ step: "agent", kind: "opencode" });
    await submit({ step: "model", model: "anthropic/claude-sonnet-4" });
    await submit({
      step: "commit",
      title: "billing-service",
      description: "Implement the billing service.",
      visibility: "private",
      name: "billing-service",
      provider_id: "prov-1",
      provider_kind: "github",
      provider_host: "github.com",
      namespace_id: "octocat",
      namespace_kind: "personal",
      namespace_name: "octocat",
      machine_kind: "local",
      machine_id: null,
      agent_kind: "opencode",
      model: "anthropic/claude-sonnet-4",
    });

    expect(submitCalls.map((c) => c.payload.step)).toEqual([
      "name",
      "provider",
      "group",
      "machine",
      "agent",
      "model",
      "commit",
    ]);

    expect(result.current.outcome?.kind).toBe("launched");
    expect(result.current.launched).toMatchObject({
      feature_id: LAUNCHED.feature_id,
      project_id: LAUNCHED.project_id,
      feature_title: LAUNCHED.feature_title,
    });
    expect(result.current.isLast).toBe(true);

    // The post-launch destination is the feature's detail page. Emitting the
    // wizard's own `create-project` surface would silently re-enter the wizard.
    expect(launchedView).toEqual({
      kind: "detail",
      featureId: LAUNCHED.feature_id,
      featureTitle: LAUNCHED.feature_title,
    });
    expect(isCreateProjectLaunchedView(launchedView!)).toBe(true);
  });
});

describe("(b) Back never revisits an auto-progressed screen", () => {
  // History: [name, provider(auto), group(auto), machine]. Three submits are
  // enough to park the wizard on `machine` — each Continue carries the state
  // the backend auto-progressed to.
  function scriptedWizard() {
    const submits: SubmitOutcome[] = [];
    let state = makeInitialState();
    for (const next of ["provider", "group", "machine"] as const) {
      state = advance(state, next);
      submits.push({ kind: "continue", state });
    }

    return makeDeps({
      submits,
      backs: [
        { step: "group", history: ["name", "provider", "group"] },
        { step: "provider", history: ["name", "provider"] },
        { step: "name", history: ["name"] },
      ],
    });
  }

  async function parkOnMachine() {
    const { deps, backCalls } = scriptedWizard();
    const onDismissed = vi.fn();
    const wizard = mountHook(deps, { onDismissed });

    await wizard.submit({ step: "name", value: "my-repo" });
    await wizard.submit({ step: "provider", provider_id: "prov-1", kind: "github" });
    await wizard.submit({
      step: "group",
      namespace_id: "octocat",
      kind: "personal",
      name: "octocat",
    });

    return { ...wizard, backCalls, onDismissed };
  }

  it("parks on machine with the auto-progressed steps recorded in history", async () => {
    const { result } = await parkOnMachine();

    expect(result.current.state.step).toBe("machine");
    expect(result.current.state.history).toEqual(["name", "provider", "group", "machine"]);
    expect(result.current.isFirst).toBe(false);
  });

  it("rewinds one history entry per Back, including the auto-progressed ones", async () => {
    const { result, back, backCalls } = await parkOnMachine();

    await back();
    expect(result.current.state.step).toBe("group");
    expect(result.current.state.history).toContain("group");

    await back();
    expect(result.current.state.step).toBe("provider");

    await back();
    expect(result.current.state.step).toBe("name");
    expect(result.current.state.history).toEqual(["name"]);
    expect(result.current.isFirst).toBe(true);

    // One IPC call per history entry. A counter-based Back would have made
    // fewer, silently skipping the auto-progressed screens.
    expect(backCalls).toHaveLength(3);
  });

  it("dismisses instead of rewinding when Back is pressed on the first step", async () => {
    const { back, onDismissed } = await parkOnMachine();

    await back();
    await back();
    await back();
    expect(onDismissed).not.toHaveBeenCalled();

    await back();

    expect(onDismissed).toHaveBeenCalledTimes(1);
  });
});

describe("(c) the post-launch view contract", () => {
  it("derives a detail view from a launched feature", () => {
    const view = viewForLaunchedFeature(LAUNCHED) as AppView;

    expect(view.kind).toBe("detail");
    expect(isCreateProjectLaunchedView(view)).toBe(true);
  });

  // The negative half: only `detail` counts as the post-launch destination.
  it.each([
    { kind: "home" },
    { kind: "new-project" },
    { kind: "create-project" },
    { kind: "empty-state" },
  ] as AppView[])("rejects the $kind view", (sample) => {
    expect(isCreateProjectLaunchedView(sample)).toBe(false);
  });
});

describe("the IPC seam", () => {
  it("surfaces an error and does not advance on a rejected submit", async () => {
    const expectedError: AppError = {
      kind: "validation",
      message: "Repository name is required",
    };
    const { deps } = makeDeps({
      submits: [
        () => {
          throw expectedError;
        },
      ],
    });

    const { result, submit } = mountHook(deps);
    await submit({ step: "name", value: "x" });

    expect(result.current.error).toMatchObject(expectedError);
    expect(result.current.state.step).toBe("name");
  });

  it("mirrors a Continue outcome so the next render shows the new step", async () => {
    const { deps } = makeDeps({
      submits: [
        { kind: "continue", state: { step: "provider", history: ["name", "provider"] } },
      ],
    });

    const { result, submit } = mountHook(deps);
    await submit({ step: "name", value: "ok-name" });

    expect(result.current.state.step).toBe("provider");
    expect(result.current.state.history).toEqual(["name", "provider"]);
    expect(result.current.isLast).toBe(false);
  });
});
