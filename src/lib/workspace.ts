import { invoke } from "@tauri-apps/api/core";

/** The directory Demeteo actually clones into right now: the override if one
 *  is set, otherwise the platform app-data directory. */
export async function getWorkspaceDir(): Promise<string> {
  return invoke<string>("get_workspace_dir");
}

/** The stored override alone, or `null` when the default is in use. */
export async function getWorkspaceDirSetting(): Promise<string | null> {
  return invoke<string | null>("get_workspace_dir_setting");
}

/** Persist an override; `null` (or blank) clears it. Takes effect on the
 *  next app start — existing projects stay where they are until they are
 *  re-bootstrapped. */
export async function setWorkspaceDirSetting(path: string | null): Promise<void> {
  return invoke<void>("set_workspace_dir_setting", { path });
}
