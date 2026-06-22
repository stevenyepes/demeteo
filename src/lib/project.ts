import { invoke } from "@tauri-apps/api/core";
import type { ProjectMemoryEntry } from "../types";

export async function listProjectMemory(projectId: string): Promise<ProjectMemoryEntry[]> {
  return invoke<ProjectMemoryEntry[]>("project_memory_list", { projectId });
}

export async function upsertProjectMemory(
  projectId: string,
  key: string,
  value: string,
  source: 'agent' | 'human',
  id?: string | null,
): Promise<void> {
  return invoke<void>("project_memory_upsert", {
    id: id || null,
    projectId,
    key,
    value,
    source,
  });
}

export async function deleteProjectMemory(id: string): Promise<void> {
  return invoke<void>("project_memory_delete", { id });
}
