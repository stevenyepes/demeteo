import { invoke } from "@tauri-apps/api/core";

export async function getRunInBackground(): Promise<boolean> {
  return invoke<boolean>("get_run_in_background");
}

export async function setRunInBackground(enabled: boolean): Promise<void> {
  return invoke<void>("set_run_in_background", { enabled });
}
