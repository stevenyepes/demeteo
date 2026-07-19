import { invoke, Channel } from "@tauri-apps/api/core";

import type { SessionInfo } from "../types";

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
 * @returns A promise that resolves to the session_id string.
 */
export async function startTerminalSession(
  machineId: string,
  workDir?: string,
  workBranch?: string | null,
  size?: { cols: number; rows: number }
): Promise<string> {
  return invoke<string>("start_terminal_session", {
    machineId,
    workDir: workDir || null,
    workBranch: workBranch ?? null,
    cols: size?.cols ?? null,
    rows: size?.rows ?? null,
  });
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
