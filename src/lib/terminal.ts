import { invoke, Channel } from "@tauri-apps/api/core";

/**
 * Starts a terminal session on the specified machine.
 * 
 * @param machineId The identifier of the machine (local or remote).
 * @param channel The Tauri IPC channel to stream stdout back to the frontend.
 * @param workDir An optional path to initialize the shell's working directory.
 * @returns A promise that resolves to the session_id string.
 */
export async function startTerminalSession(
  machineId: string,
  channel: Channel<Uint8Array | number[]>,
  workDir?: string
): Promise<string> {
  return invoke<string>("start_terminal_session", {
    machineId,
    tauriChannel: channel,
    workDir: workDir || null,
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
