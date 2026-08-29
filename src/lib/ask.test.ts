import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AskThread, AskThreadDetail } from "../types";
import {
  createAskThread,
  deleteAskThread,
  listAskThreads,
  loadAskThread,
  renameAskThread,
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
      },
    });
  });

  it("passes null model/effort/machineId through unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(THREAD);

    await createAskThread({
      projectId: "project-1",
      title: "Ask about auth",
      agentKind: "claude-code",
      model: null,
      effort: null,
      machineId: null,
    });

    expect(invoke).toHaveBeenCalledWith("ask_create", {
      input: {
        project_id: "project-1",
        title: "Ask about auth",
        agent_kind: "claude-code",
        model: null,
        effort: null,
        machine_id: null,
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
