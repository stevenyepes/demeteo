import { invoke } from "@tauri-apps/api/core";
import type {
  ProjectMemoryEntry,
  MemoryAgentConfig,
  MemoryAgentTestResult,
} from "../types";

/**
 * Resolve the machine id used to probe a project's workspace connectivity.
 * Mirrors the resolution logic previously duplicated server-side in the
 * (now-removed) `check_liveness` command: a `local` (or missing)
 * `compute_type` always resolves to `'local'`; anything else resolves to
 * the project's `remote_host` (falling back to `'local'` if unset).
 */
export function deriveMachineId(project: {
  compute_type?: string;
  remote_host?: string | null;
}): string {
  if (!project.compute_type || project.compute_type.toLowerCase() === "local") {
    return "local";
  }
  return project.remote_host ?? "local";
}

/**
 * Probe whether the machine backing a project's workspace is currently
 * reachable, by reusing the `test_machine_connection` IPC (the same probe
 * `MachinesView.tsx` and `useCreateZeroWizardForm.ts` use). Never rejects —
 * a failed probe resolves with `liveness: 'offline'` so callers don't need
 * a catch branch.
 */
export async function checkWorkspaceLiveness(project: {
  id: string;
  compute_type?: string;
  remote_host?: string | null;
}): Promise<{ project_id: string; liveness: "online" | "offline"; checked_at: string }> {
  try {
    await invoke("test_machine_connection", { machineId: deriveMachineId(project) });
    return { project_id: project.id, liveness: "online", checked_at: new Date().toISOString() };
  } catch {
    return { project_id: project.id, liveness: "offline", checked_at: new Date().toISOString() };
  }
}

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

// ── Memory agent (global) ──────────────────────────────────────────────

export async function getMemoryAgentConfig(): Promise<MemoryAgentConfig> {
  return invoke<MemoryAgentConfig>("memory_agent_config_get");
}

/** Persist config. `apiKey`: `undefined` keeps the stored key, `''` clears it,
 * a non-empty string stores a new key. */
export async function setMemoryAgentConfig(
  config: MemoryAgentConfig,
  apiKey?: string,
): Promise<void> {
  return invoke<void>("memory_agent_config_set", {
    config,
    apiKey: apiKey === undefined ? null : apiKey,
  });
}

export async function testMemoryAgentConnection(
  config: MemoryAgentConfig,
  apiKey?: string,
): Promise<MemoryAgentTestResult> {
  return invoke<MemoryAgentTestResult>("memory_agent_test_connection", {
    config,
    apiKey: apiKey === undefined ? null : apiKey,
  });
}

/** List models available at an endpoint (OpenAI `/models`, falling back to
 * Ollama `/api/tags`). */
export async function listMemoryAgentModels(
  endpoint: string,
  apiKey?: string,
): Promise<string[]> {
  return invoke<string[]>("memory_agent_list_models", {
    endpoint,
    apiKey: apiKey === undefined ? null : apiKey,
  });
}

/**
 * Partial project-settings input. Any field left `undefined` is filled from
 * the existing DB record (or a sensible default). This prevents the
 * partial-save data-loss bug where a caller that omits a field would
 * accidentally `INSERT OR REPLACE` it to NULL.
 */
export interface ProjectSettingsInput {
  default_branch?: string;
  branch_prefix?: string;
  test_command?: string | null;
  build_command?: string | null;
  coverage_command?: string | null;
  conventions_file?: string | null;
  pr_template?: string | null;
  harnesses?: Record<string, string> | null;
  prepare_command?: string | null;
  extra_writable_paths?: string[] | null;
  conflict_policy?: string;
  feature_lifecycle?: string;
  default_agent_kind?: string | null;
  default_model?: string | null;
  default_loop_iterations?: number | null;
  artifact_subdir?: string;
  commit_artifacts?: boolean;
}

/**
 * Read existing DB settings, overlay the caller's partial input, and write
 * back the complete merged record.  Any field omitted from `input` (i.e.
 * left `undefined`) keeps whatever is already in the database, so a save
 * call that only touches a few form fields can never NULL out the rest.
 */
export async function saveProjectSettings(
  projectId: string,
  input: ProjectSettingsInput,
): Promise<void> {
  const existing = await invoke<any | null>("get_proposed_strategy", {
    projectId,
  });
  const baseWs = existing?.worktree_strategy;

  const merged = {
    project_id: projectId,
    worktree_strategy: {
      default_branch:
        input.default_branch ?? baseWs?.default_branch ?? "main",
      branch_prefix:
        input.branch_prefix ?? baseWs?.branch_prefix ?? "demeteo/features/",
      test_command:
        input.test_command !== undefined
          ? input.test_command
          : (baseWs?.test_command ?? null),
      build_command:
        input.build_command !== undefined
          ? input.build_command
          : (baseWs?.build_command ?? null),
      coverage_command:
        input.coverage_command !== undefined
          ? input.coverage_command
          : (baseWs?.coverage_command ?? null),
      conventions_file:
        input.conventions_file !== undefined
          ? input.conventions_file
          : (baseWs?.conventions_file ?? null),
      pr_template:
        input.pr_template !== undefined
          ? input.pr_template
          : (baseWs?.pr_template ?? null),
      harnesses:
        input.harnesses !== undefined
          ? (Object.keys(input.harnesses ?? {}).length > 0
              ? input.harnesses
              : null)
          : (baseWs?.harnesses ?? null),
      prepare_command:
        input.prepare_command !== undefined
          ? input.prepare_command
          : (baseWs?.prepare_command ?? null),
      extra_writable_paths:
        input.extra_writable_paths !== undefined
          ? (Array.isArray(input.extra_writable_paths)
              ? input.extra_writable_paths
              : [])
          : (baseWs?.extra_writable_paths ?? []),
    },
    conflict_policy:
      input.conflict_policy ?? existing?.conflict_policy ?? "always_gate",
    feature_lifecycle:
      input.feature_lifecycle ?? existing?.feature_lifecycle ?? "archive",
    default_agent_kind:
      input.default_agent_kind !== undefined
        ? input.default_agent_kind
        : (existing?.default_agent_kind ?? null),
    default_model:
      input.default_model !== undefined
        ? input.default_model
        : (existing?.default_model ?? null),
    default_loop_iterations:
      input.default_loop_iterations !== undefined
        ? input.default_loop_iterations
        : (existing?.default_loop_iterations ?? null),
    artifact_subdir:
      input.artifact_subdir ?? existing?.artifact_subdir ?? "artifacts/",
    commit_artifacts:
      input.commit_artifacts ?? existing?.commit_artifacts ?? false,
  };

  await invoke("save_project_settings", { projectId, settings: merged });
}
