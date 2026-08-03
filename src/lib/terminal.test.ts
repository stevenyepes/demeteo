// Unit tests for the terminal IPC wrappers added in spec §2.1 / §4.
//
// The wrappers are intentionally thin: each one is a single `invoke<...>(...)`
// call with a snake_case command name and a camelCase args object. These
// tests guard the wire contract against accidental typos — a typo would
// silently fail the IPC under jsdom (where `invoke` is mocked to a no-op),
// so without these tests a wrong command name would compile and ship and
// only surface at runtime.

import { Channel, invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  CreatedTerminalWorktree,
  CreateTerminalWorktreeRequest,
  SessionInfo,
  TerminalBranchOptions,
  TerminalWorktree,
} from "../types";
import {
  attachTerminalSession,
  createTerminalWorktree,
  detachTerminalSession,
  listTerminalBranches,
  listTerminalWorktrees,
  listTerminalSessions,
  removeTerminalWorktree,
  renameTerminalSession,
} from "./terminal";

beforeEach(() => {
  // `setup.ts` installs a default implementation that returns `[]` for
  // `list_terminal_sessions` and `undefined` for everything else; per-test
  // overrides stack on top via `mockResolvedValueOnce`.
  vi.mocked(invoke).mockReset();
});

// Minimal `Channel` stub. The wrappers never call any method on the
// channel — they just hand it through to `invoke` as `tauriChannel` — so
// an empty object satisfies the runtime contract. The cast keeps
// TypeScript happy without needing the real `Channel` constructor (which
// expects a `window.postMessage` bridge that jsdom does not provide).
function stubChannel(): Channel<Uint8Array | number[]> {
  return {} as unknown as Channel<Uint8Array | number[]>;
}

describe("attachTerminalSession", () => {
  it("forwards sessionId and the channel to attach_terminal_session", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    const channel = stubChannel();

    await attachTerminalSession("sess_abc", channel);

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("attach_terminal_session", {
      sessionId: "sess_abc",
      tauriChannel: channel,
    });
  });

  it("does not include extra args (no work_dir, no work_branch)", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);

    await attachTerminalSession("sess_xyz", stubChannel());

    const [, args] = vi.mocked(invoke).mock.calls[0];
    expect(Object.keys(args as Record<string, unknown>)).toEqual([
      "sessionId",
      "tauriChannel",
    ]);
  });

  it("propagates a backend rejection instead of swallowing it", async () => {
    vi.mocked(invoke).mockRejectedValueOnce("Session not found");

    await expect(
      attachTerminalSession("sess_missing", stubChannel()),
    ).rejects.toBe("Session not found");
  });
});

describe("detachTerminalSession", () => {
  it("forwards sessionId to detach_terminal_session", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);

    await detachTerminalSession("sess_xyz");

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("detach_terminal_session", {
      sessionId: "sess_xyz",
    });
  });

  it("propagates a backend rejection so the caller can react", async () => {
    vi.mocked(invoke).mockRejectedValueOnce("Session not found");

    await expect(detachTerminalSession("sess_missing")).rejects.toBe(
      "Session not found",
    );
  });
});

describe("listTerminalSessions", () => {
  it("invokes list_terminal_sessions with no args", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([]);

    const result = await listTerminalSessions();

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("list_terminal_sessions");
    expect(result).toEqual([]);
  });

  it("returns the SessionInfo payload the backend emits", async () => {
    const payload = [
      { session_id: "s1", machine_id: "local", created_at: 100, title: null },
      { session_id: "s2", machine_id: "remote", created_at: 200, title: "build" },
    ] satisfies SessionInfo[];
    vi.mocked(invoke).mockResolvedValueOnce(payload);

    const result = await listTerminalSessions();

    expect(result).toEqual(payload);
  });
});

describe("listTerminalWorktrees", () => {
  it("lists a project repository's worktrees and maps the wire lock field", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([
      { path: "/repos/app", branch: "main", is_locked: false },
      { path: "/repos/app/feature", branch: null, is_locked: true },
    ]);

    const result = await listTerminalWorktrees("project_1", "repository_1");

    expect(invoke).toHaveBeenCalledWith("list_terminal_worktrees", {
      projectId: "project_1",
      repositoryId: "repository_1",
    });
    expect(result).toEqual([
      { path: "/repos/app", branch: "main", isLocked: false },
      { path: "/repos/app/feature", branch: null, isLocked: true },
    ] satisfies TerminalWorktree[]);
  });
});

describe("listTerminalBranches", () => {
  it("maps the wire flags that say whether a base can be refreshed", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      default_branch: "main",
      branches: [
        { name: "main", has_local: true, has_remote: true },
        { name: "scratch", has_local: true, has_remote: false },
      ],
    });

    const result = await listTerminalBranches("project_1", "repository_1");

    expect(invoke).toHaveBeenCalledWith("list_terminal_branches", {
      projectId: "project_1",
      repositoryId: "repository_1",
    });
    expect(result).toEqual({
      defaultBranch: "main",
      branches: [
        { name: "main", hasLocal: true, hasRemote: true },
        { name: "scratch", hasLocal: true, hasRemote: false },
      ],
    } satisfies TerminalBranchOptions);
  });
});

describe("createTerminalWorktree", () => {
  it("forwards only the create request fields and maps the result", async () => {
    const request = {
      projectId: "project_1",
      repositoryId: "repository_1",
      branch: "feature/terminal",
      baseBranch: "main",
      worktreeName: "terminal-feature",
    } satisfies CreateTerminalWorktreeRequest;
    vi.mocked(invoke).mockResolvedValueOnce({
      worktree: {
        path: "/repos/app/.worktrees/terminal-feature",
        branch: "feature/terminal",
        is_locked: false,
      },
      base_ref: "origin/main",
    });

    const result = await createTerminalWorktree(request);

    expect(invoke).toHaveBeenCalledWith("create_terminal_worktree", request);
    expect(result).toEqual({
      worktree: {
        path: "/repos/app/.worktrees/terminal-feature",
        branch: "feature/terminal",
        isLocked: false,
      },
      baseRef: "origin/main",
    } satisfies CreatedTerminalWorktree);
  });
});

describe("removeTerminalWorktree", () => {
  it("forwards the path and the force decision verbatim", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(null);

    await removeTerminalWorktree("project_1", "repository_1", "/repos/app/.worktrees/gone", true);

    expect(invoke).toHaveBeenCalledWith("remove_terminal_worktree", {
      projectId: "project_1",
      repositoryId: "repository_1",
      worktreePath: "/repos/app/.worktrees/gone",
      force: true,
    });
  });
});

describe("renameTerminalSession", () => {
  it("forwards sessionId and title to rename_terminal_session", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);

    await renameTerminalSession("sess_1", "build");

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("rename_terminal_session", {
      sessionId: "sess_1",
      title: "build",
    });
  });

  it("forwards an empty title verbatim — the backend owns the clear semantics", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);

    await renameTerminalSession("sess_1", "");

    expect(invoke).toHaveBeenCalledWith("rename_terminal_session", {
      sessionId: "sess_1",
      title: "",
    });
  });

  it("propagates a backend rejection so the caller can revert the title", async () => {
    vi.mocked(invoke).mockRejectedValueOnce("Session not found");

    await expect(
      renameTerminalSession("sess_missing", "build"),
    ).rejects.toBe("Session not found");
  });
});
