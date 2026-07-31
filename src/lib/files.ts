import { invoke } from "@tauri-apps/api/core";

/** One directory entry as the execution transport reports it. Mirrors the
 *  Rust `SftpEntry`; the same shape comes back for a local machine, which is
 *  what lets the code editor ignore where the worktree lives. */
export interface SftpEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
}

export async function listDir(machineId: string, path: string): Promise<SftpEntry[]> {
  return invoke<SftpEntry[]>("sftp_list_dir", { machineId, path });
}

/**
 * Read a file for general browsing. Run-artifact display uses
 * `artifactBody` instead — that read goes through `RunView` so a
 * runner-owned artifact can resolve from the laptop shadow.
 */
export async function readFile(machineId: string, path: string): Promise<string> {
  return invoke<string>("sftp_read_file", { machineId, path });
}

/** A file changed between two refs. `status` is git's letter: `M | A | D |
 *  R | ?`. Mirrors the Rust `ChangedFile`. */
export interface ChangedFile {
  path: string;
  status: string;
}

export async function gitChangedFiles(input: {
  machineId: string;
  worktreePath: string;
  baseRef: string;
  headRef: string;
}): Promise<ChangedFile[]> {
  return invoke<ChangedFile[]>("git_changed_files", {
    machineId: input.machineId,
    worktreePath: input.worktreePath,
    baseRef: input.baseRef,
    headRef: input.headRef,
  });
}

export async function gitFileAtRef(input: {
  machineId: string;
  worktreePath: string;
  gitRef: string;
  filePath: string;
}): Promise<string> {
  return invoke<string>("git_file_at_ref", {
    machineId: input.machineId,
    worktreePath: input.worktreePath,
    gitRef: input.gitRef,
    filePath: input.filePath,
  });
}
