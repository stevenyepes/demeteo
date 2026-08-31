import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AskThread, AskThreadDetail } from "../types";
import {
  createAskThread,
  deleteAskThread,
  listAskThreads,
  loadAskThread,
  renameAskThread,
  updateAskThreadSettings,
} from "./ask";

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

const THREAD: AskThread = {
  id: "thread-1",
  project_id: "project-1",
  title: "Ask about auth",
  status: "open",
  agent_kind: "claude-code",
  model: null,
  effort: null,
  machine_id: "local",
  worktree_path: null,
  session_id: null,
  turn_count: 0,
  cost_usd: 0,
  tokens: 0,
  network: true,
  created_at: 1,
  updated_at: 1,
};

describe("createAskThread", () => {
  it("translates camelCase input into the Rust snake_case payload", async () => {
    vi.mocked(invoke).mockResolvedValue(THREAD);

    const result = await createAskThread({
      projectId: "project-1",
      title: "Ask about auth",
      agentKind: "claude-code",
      model: "sonnet",
      effort: "medium",
      machineId: "machine-1",
      network: true,
    });

    expect(result).toBe(THREAD);
    expect(invoke).toHaveBeenCalledWith("ask_create", {
      input: {
        project_id: "project-1",
        title: "Ask about auth",
        agent_kind: "claude-code",
        model: "sonnet",
        effort: "medium",
        machine_id: "machine-1",
        network: true,
      },
    });
  });

  it("passes null model/effort/machineId and the network posture through unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(THREAD);

    await createAskThread({
      projectId: "project-1",
      title: "Ask about auth",
      agentKind: "claude-code",
      model: null,
      effort: null,
      machineId: null,
      network: false,
    });

    expect(invoke).toHaveBeenCalledWith("ask_create", {
      input: {
        project_id: "project-1",
        title: "Ask about auth",
        agent_kind: "claude-code",
        model: null,
        effort: null,
        machine_id: null,
        network: false,
      },
    });
  });
});

describe("listAskThreads", () => {
  it("mirrors ask_list with the project id", async () => {
    vi.mocked(invoke).mockResolvedValue([THREAD]);

    const result = await listAskThreads("project-1");

    expect(result).toEqual([THREAD]);
    expect(invoke).toHaveBeenCalledWith("ask_list", { projectId: "project-1" });
  });
});

describe("loadAskThread", () => {
  it("mirrors ask_load with the thread id", async () => {
    const detail: AskThreadDetail = { thread: THREAD, messages: [] };
    vi.mocked(invoke).mockResolvedValue(detail);

    const result = await loadAskThread("thread-1");

    expect(result).toEqual(detail);
    expect(invoke).toHaveBeenCalledWith("ask_load", { threadId: "thread-1" });
  });
});

describe("renameAskThread", () => {
  it("mirrors ask_rename with the thread id and title", async () => {
    vi.mocked(invoke).mockResolvedValue({ ...THREAD, title: "Renamed" });

    const result = await renameAskThread("thread-1", "Renamed");

    expect(result.title).toBe("Renamed");
    expect(invoke).toHaveBeenCalledWith("ask_rename", {
      threadId: "thread-1",
      title: "Renamed",
    });
  });
});

describe("deleteAskThread", () => {
  it("mirrors ask_delete with the thread id", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await deleteAskThread("thread-1");

    expect(invoke).toHaveBeenCalledWith("ask_delete", { threadId: "thread-1" });
  });
});

/**
 * `toHaveBeenCalledWith` compares with `toEqual`, which reads an explicitly
 * `undefined` property as absent — the exact distinction these tests exist to
 * hold. So they read the sent keys directly.
 */
function sentPatch(): Record<string, unknown> {
  const [, args] = vi.mocked(invoke).mock.calls[0] as [string, { patch: Record<string, unknown> }];
  return args.patch;
}

describe("updateAskThreadSettings", () => {
  it("omits a key the caller left out, so the column is left alone", async () => {
    vi.mocked(invoke).mockResolvedValue(THREAD);

    await updateAskThreadSettings("thread-1", { network: false });

    expect(Object.keys(sentPatch())).toEqual(["network"]);
    expect(invoke).toHaveBeenCalledWith("ask_update_settings", {
      threadId: "thread-1",
      patch: { network: false },
    });
  });

  it("sends an explicit null, which the Rust side reads as a clear", async () => {
    vi.mocked(invoke).mockResolvedValue(THREAD);

    await updateAskThreadSettings("thread-1", { model: null, effort: null });

    const patch = sentPatch();
    expect(Object.keys(patch).sort()).toEqual(["effort", "model"]);
    expect(patch.model).toBeNull();
    expect(patch.effort).toBeNull();
  });

  it("passes chosen values through unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue({ ...THREAD, model: "sonnet", effort: "high" });

    const result = await updateAskThreadSettings("thread-1", {
      model: "sonnet",
      effort: "high",
    });

    expect(result.model).toBe("sonnet");
    expect(invoke).toHaveBeenCalledWith("ask_update_settings", {
      threadId: "thread-1",
      patch: { model: "sonnet", effort: "high" },
    });
  });
});
