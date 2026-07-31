import { invoke, Channel } from "@tauri-apps/api/core";

import type {
  CreateTerminalWorktreeRequest,
  SessionInfo,
  TerminalWorktree,
} from "../types";

/** Rust's `WorktreeInfo` IPC shape. Keep this wire detail in this module. */
interface TerminalWorktreeWire {
  path: string;
  branch: string | null;
  is_locked: boolean;
}

function toTerminalWorktree(worktree: TerminalWorktreeWire): TerminalWorktree {
  return {
    path: worktree.path,
    branch: worktree.branch,
    isLocked: worktree.is_locked,
  };
}

/**
 * Starts a terminal session on the specified machine.
 *
 * The session streams no output until a surface calls
 * `attachTerminalSession`. Output produced before the first attach (shell
 * startup, the `git checkout` bootstrap, the first prompt) accumulates in
 * the backend scrollback ring and is replayed to the first channel that
 * attaches — so nothing is lost across the start→attach gap and no seed
 * channel is required (TERMINALS_VIEW_SPEC §3).
 *
 * @param machineId The identifier of the machine (local or remote).
 * @param workDir An optional path to initialize the shell's working directory.
 * @param workBranch An optional feature branch to `git checkout` after the
 *   shell starts. When supplied, the backend appends a
 *   `git checkout <branch> 2>/dev/null || git switch <branch> 2>/dev/null`
 *   bootstrap so the PTY opens on that branch. Missing-branch failures are
 *   swallowed silently (the user still gets a usable terminal).
 * @param size An optional initial `{ cols, rows }` so the shell draws its very
 *   first prompt at (near) the real terminal width. When omitted the backend
 *   falls back to a conservative 80x24 (narrower than any viewport, so the
 *   first prompt never wraps); the surface still sends the exact size via
 *   `resizeTerminalSession` once it mounts and fits.
 * @param agentKind An optional coding-agent kind (e.g. `"claude-code"`) being
 *   launched into the fresh session. Seeds the session's agent label so the
 *   tab shows the badge immediately; for local sessions the backend's
 *   foreground detector keeps it accurate afterwards.
 * @param launchCommand The base command the caller intends to write into the
 *   fresh session (e.g. `"claude"`). Handed to the backend so that, for hooked
 *   agent kinds, it can return an augmented launch line (base +
 *   `--settings '<reporter hooks>'`). The caller then writes the returned
 *   `launchCommand` instead of its own — the single-write contract that stops
 *   the agent launching twice (TERMINAL_ACTIVITY §2c).
 * @returns A promise resolving to `{ sessionId, launchCommand }`. `launchCommand`
 *   is the backend's augmented launch line for hooked kinds, else `null`.
 */
export async function startTerminalSession(
  machineId: string,
  workDir?: string,
  workBranch?: string | null,
  size?: { cols: number; rows: number },
  agentKind?: string | null,
  launchCommand?: string | null
): Promise<{ sessionId: string; launchCommand: string | null }> {
  // The backend returns a `StartedSession` struct (serde snake_case:
  // `session_id`, `launch_command`). A bare-string return is tolerated so
  // legacy test mocks and any older invoke path still resolve to a session id.
  const result = await invoke<
    { session_id: string; launch_command: string | null } | string
  >("start_terminal_session", {
    machineId,
    workDir: workDir || null,
    workBranch: workBranch ?? null,
    cols: size?.cols ?? null,
    rows: size?.rows ?? null,
    agentKind: agentKind ?? null,
    launchCommand: launchCommand ?? null,
  });
  if (typeof result === "string") {
    return { sessionId: result, launchCommand: null };
  }
  return {
    sessionId: result.session_id,
    launchCommand: result.launch_command ?? null,
  };
}

/**
 * Writes data into the terminal session's standard input.
 */
export async function writeTerminalSession(
  sessionId: string,
  data: string
): Promise<void> {
  return invoke<void>("write_terminal_session", {
    sessionId,
    data,
  });
}

/**
 * Resizes the PTY of the terminal session.
 */
export async function resizeTerminalSession(
  sessionId: string,
  cols: number,
  rows: number
): Promise<void> {
  return invoke<void>("resize_terminal_session", {
    sessionId,
    cols,
    rows,
  });
}

/**
 * Closes the active terminal session.
 */
export async function closeTerminalSession(sessionId: string): Promise<void> {
  return invoke<void>("close_terminal_session", {
    sessionId,
  });
}

/**
 * Resolves the absolute directory path of a repository within a project.
 */
export async function resolveRepoDir(
  projectId: string,
  repoPath: string
): Promise<string> {
  return invoke<string>("resolve_repo_dir", {
    projectId,
    repoPath,
  });
}

/** Lists linked worktrees available for a repository owned by a project. */
export async function listTerminalWorktrees(
  projectId: string,
  repositoryId: string,
): Promise<TerminalWorktree[]> {
  const worktrees = await invoke<TerminalWorktreeWire[]>("list_terminal_worktrees", {
    projectId,
    repositoryId,
  });
  return worktrees.map(toTerminalWorktree);
}

/** Creates a linked worktree using a project-owned repository destination. */
export async function createTerminalWorktree(
  request: CreateTerminalWorktreeRequest,
): Promise<TerminalWorktree> {
  const worktree = await invoke<TerminalWorktreeWire>(
    "create_terminal_worktree",
    {
      projectId: request.projectId,
      repositoryId: request.repositoryId,
      branch: request.branch,
      worktreeName: request.worktreeName,
    },
  );
  return toTerminalWorktree(worktree);
}

/**
 * Attaches an output channel to an active terminal session.
 *
 * @param sessionId The terminal session identifier.
 * @param channel The Tauri IPC channel that receives terminal output.
 * @returns A promise that resolves when the channel is attached.
 */
export async function attachTerminalSession(
  sessionId: string,
  channel: Channel<Uint8Array | number[]>
): Promise<void> {
  return invoke<void>("attach_terminal_session", {
    sessionId,
    tauriChannel: channel,
  });
}

/**
 * Detaches output channels from an active terminal session.
 *
 * @param sessionId The terminal session identifier.
 * @param channelId The Tauri `Channel.id` of the subscriber to remove.
 *   When supplied, only the matching subscriber is evicted — this is
 *   race-safe against a fresh `attach_terminal_session` call that
 *   happens to be re-binding the same session. When omitted, the
 *   backend falls back to LIFO-pop semantics (V1 single-subscriber).
 * @returns A promise that resolves when the channels are detached.
 */
export async function detachTerminalSession(
  sessionId: string,
  channelId?: number
): Promise<void> {
  const args: { sessionId: string; channelId?: number } = { sessionId };
  if (channelId !== undefined) {
    args.channelId = channelId;
  }
  return invoke<void>("detach_terminal_session", args);
}

/**
 * Lists active terminal sessions.
 *
 * @returns A promise that resolves to the active sessions.
 */
export async function listTerminalSessions(): Promise<SessionInfo[]> {
  return invoke<SessionInfo[]>("list_terminal_sessions");
}

/**
 * Renames an active terminal session.
 *
 * @param sessionId The terminal session identifier.
 * @param title The new terminal session title.
 * @returns A promise that resolves when the title is stored.
 */
export async function renameTerminalSession(
  sessionId: string,
  title: string
): Promise<void> {
  return invoke<void>("rename_terminal_session", {
    sessionId,
    title,
  });
}

/**
 * Re-establishes the transport for a disconnected terminal session. The
 * session shell — id, scrollback, subscribers, title — survived the drop;
 * the backend spawns a fresh PTY/SSH child on the same session, replays
 * scrollback as history, and emits `terminal-session-running`
 * (TERMINALS_VIEW_SPEC §3.1). Rejects if the session id is unknown or the
 * session is still connected.
 *
 * @param sessionId The disconnected terminal session identifier.
 * @returns A promise that resolves once the transport is rebuilt.
 */
export async function reconnectTerminalSession(
  sessionId: string
): Promise<void> {
  return invoke<void>("reconnect_terminal_session", {
    sessionId,
  });
}

/**
 * Report to the backend whether the on-screen recognizer (Phase 3) currently
 * sees an agent's approval prompt rendered in a session. `present = true`
 * latches a screen-sourced `awaiting_approval`; `present = false` retracts it.
 * The backend folds this into the same activity record as the cadence sweep and
 * the Claude hook scanner, so precedence, dedup, and the OS notification are
 * reused — a screen-sourced approval behaves exactly like the hook-sourced one
 * (TERMINAL_ACTIVITY §Phase 3). Fire-and-forget from the recognizer's
 * debounce; the resolved state comes back over `terminal-session-activity`.
 *
 * @param sessionId The backend terminal session identifier.
 * @param present   Whether an approval prompt is on screen right now.
 */
export async function reportScreenActivity(
  sessionId: string,
  present: boolean
): Promise<void> {
  return invoke<void>("report_terminal_screen_activity", {
    sessionId,
    present,
  });
}
