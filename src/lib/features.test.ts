import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { isEnvironmentError, remoteSetStepAssignment, setStepAssignment } from "./features";

describe("isEnvironmentError", () => {
  it("matches the backend's terminal environment failure", () => {
    const msg =
      "Environment not ready — this failure is not something editing the code can fix.\n\n" +
      "The shell could not find `cargo` on PATH (exit 127), so the command never ran.\n";
    expect(isEnvironmentError(msg)).toBe(true);
  });

  it("ignores an ordinary step failure", () => {
    expect(isEnvironmentError("thread 'main' panicked at src/main.rs:4:5")).toBe(false);
  });

  it("treats a missing error message as not an environment failure", () => {
    expect(isEnvironmentError(null)).toBe(false);
    expect(isEnvironmentError(undefined)).toBe(false);
    expect(isEnvironmentError("")).toBe(false);
  });
});

describe("assignment wrappers", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
  });

  it("sends the local pin under the command and argument names the Tauri command declares", async () => {
    await setStepAssignment({
      stepExecutionId: "se-1",
      agentKind: "claude-code",
      model: "claude-opus-5",
      effort: "high",
    });

    expect(invoke).toHaveBeenCalledWith("step_set_assignment", {
      stepExecutionId: "se-1",
      agentKind: "claude-code",
      model: "claude-opus-5",
      effort: "high",
    });
  });

  it("sends the detached pin with the machine and run it belongs to", async () => {
    await remoteSetStepAssignment({
      machineId: "m-1",
      runId: "f-1",
      stepExecutionId: "se-1",
      agentKind: "opencode",
      model: null,
      effort: "low",
    });

    expect(invoke).toHaveBeenCalledWith("remote_set_step_assignment", {
      machineId: "m-1",
      runId: "f-1",
      stepExecutionId: "se-1",
      agentKind: "opencode",
      model: null,
      effort: "low",
    });
  });

  // All three `null` is the reset-to-inherited request, so the wrapper has to
  // put every key on the wire. Dropping a `null` — or short-circuiting the
  // call as a no-op — would leave the step's existing pin standing, which is
  // the one outcome this argument shape cannot express any other way.
  it("puts all three keys on the wire for the all-null reset", async () => {
    await setStepAssignment({
      stepExecutionId: "se-1",
      agentKind: null,
      model: null,
      effort: null,
    });
    await remoteSetStepAssignment({
      machineId: "m-1",
      runId: "f-1",
      stepExecutionId: "se-1",
      agentKind: null,
      model: null,
      effort: null,
    });

    expect(invoke).toHaveBeenNthCalledWith(1, "step_set_assignment", {
      stepExecutionId: "se-1",
      agentKind: null,
      model: null,
      effort: null,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "remote_set_step_assignment", {
      machineId: "m-1",
      runId: "f-1",
      stepExecutionId: "se-1",
      agentKind: null,
      model: null,
      effort: null,
    });
  });
});
