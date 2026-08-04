import { invoke } from "@tauri-apps/api/core";
import type {
  EffortLevel,
  Project,
  ProjectMemoryEntry,
  ProjectSettingsData,
  MemoryAgentConfig,
  MemoryAgentTestResult,
  Repository,
  WorkflowOverride,
} from "../types";

// ── Project records ─────────────────────────────────────────────────────

export async function getProjects(): Promise<Project[]> {
  return invoke<Project[]>("get_projects");
}

/** Create the demo project. The backend pins a fixed id and discards the
 *  insert result, so a second call resolves with the same record whether or
 *  not a row was written — treat the result as the intended shape, not as
 *  proof of what is stored. */
export async function seedSampleProject(): Promise<Project> {
  return invoke<Project>("seed_sample_project");
}

export async function getRepositoriesForProject(projectId: string): Promise<Repository[]> {
  return invoke<Repository[]>("get_repositories_for_project", { projectId });
}

/** Mirrors the Rust `ProjectConfig`. Sent whole on every update, so a caller
 *  that drops `repos` drops the project's repositories. */
export interface ProjectConfigInput {
  name: string;
  compute_type: string;
  remote_host: string | null;
  repos: Array<{ repo_path: string; provider_id: string }>;
}

export async function updateProject(id: string, config: ProjectConfigInput): Promise<void> {
  return invoke<void>("update_project", { id, config });
}

/** Delete the project and its cloned workspace. */
export async function deleteProject(id: string): Promise<void> {
  return invoke<void>("delete_project", { id });
}

/** Uncommitted / unpushed state of the named repos. Mirrors the Rust
 *  `RepoDirtyStatus`. */
export interface RepoDirtyStatus {
  repo_path: string;
  has_uncommitted: boolean;
  has_unpushed: boolean;
}

export async function checkReposDirty(
  projectId: string,
  repoPaths: string[],
): Promise<RepoDirtyStatus[]> {
  return invoke<RepoDirtyStatus[]>("check_repos_dirty", { projectId, repoPaths });
}

export interface WorktreeInfo {
  path: string;
  branch: string | null;
  is_locked: boolean;
}

/** Mirrors the Rust `RepoHealthStatus`. */
export interface RepoHealthStatus {
  repo_path: string;
  is_cloned: boolean;
  head_branch: string | null;
  worktrees: WorktreeInfo[];
  has_uncommitted: boolean;
  has_unpushed: boolean;
}

export async function getWorkspaceHealth(projectId: string): Promise<RepoHealthStatus[]> {
  return invoke<RepoHealthStatus[]>("get_workspace_health", { projectId });
}

/** The project's stored settings, or `null` before its first save. */
export async function getProposedStrategy(
  projectId: string,
): Promise<ProjectSettingsData | null> {
  return invoke<ProjectSettingsData | null>("get_proposed_strategy", { projectId });
}

// ── Project memory ──────────────────────────────────────────────────────

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

// ── Workflow / step overrides ──────────────────────────────────────────

/** Every override configured for a project, workflow-level and step-level.
 *  Anything inheriting has no row and is absent from the list. */
export async function getWorkflowOverrides(projectId: string): Promise<WorkflowOverride[]> {
  return invoke<WorkflowOverride[]>("get_workflow_overrides", { projectId });
}

/**
 * Upsert one project-scoped override. `stepId === null` is the
 * workflow-level row ("applies to all steps"); a step id targets one step.
 * Each field is independently `null` = "inherit that one field"; all three
 * `null` clears the row entirely (the repo deletes it).
 */
export async function setWorkflowOverride(input: {
  projectId: string;
  workflowId: string;
  stepId: string | null;
  agentKind: string | null;
  model: string | null;
  effort: EffortLevel | null;
}): Promise<void> {
  await invoke<void>("set_workflow_override", {
    projectId: input.projectId,
    workflowId: input.workflowId,
    stepId: input.stepId,
    agentKind: input.agentKind,
    model: input.model,
    effort: input.effort,
  });
}

// ── Configuration-time command probe (HB6) ─────────────────────────────────

/** Which project setting a probed command came from. */
export type ProbedCommandSource = 'prepare' | 'test' | 'harness';

/** One binary a configured command names, and whether the machine found it.
 *  A binary the engine deliberately skipped (a shell builtin, a `$(…)`
 *  substitution, a glob) is absent from the list rather than reported as
 *  healthy — the panel claims exactly what was checked. */
export interface ProbedBinary {
  name: string;
  resolved: boolean;
}

export interface ProbedCommand {
  source: ProbedCommandSource;
  /** The `harnesses` key, for `source === 'harness'`. */
  harness: string | null;
  command: string;
  binaries: ProbedBinary[];
}

export interface CommandProbeReport {
  /** The machine that was actually asked — the *project's*, not the laptop's. */
  machine: string;
  commands: ProbedCommand[];
  /** The engine's own `PreflightVerdict::detail` — the same string a blocked
   *  launch terminates with, carrying the `bash -l -i -c` reproduce line.
   *  Rendered verbatim so the panel and the launch cannot drift apart. */
  detail: string | null;
  /** The engine's fresh-checkout / watch-mode remediation, likewise verbatim. */
  guidance: string;
  /** Whether this verdict would stop a launch. Reported, never enforced: a
   *  save is not gated on a probe. */
  blocks_launch: boolean;
}

/**
 * Probe the project's configured commands on **the project's own machine**.
 *
 * The commands are sent as they stand in the form, not read back from the DB:
 * the point is to answer for the command the user just typed. Which machine
 * gets asked is decided backend-side from the project's compute type, so the
 * panel cannot accidentally report the laptop's PATH for a remote project.
 */
export async function probeProjectCommands(input: {
  projectId: string;
  prepareCommand: string | null;
  testCommand: string | null;
  harnesses: Record<string, string> | null;
}): Promise<CommandProbeReport> {
  return invoke<CommandProbeReport>('probe_project_commands', {
    projectId: input.projectId,
    draft: {
      prepare_command: input.prepareCommand || null,
      test_command: input.testCommand || null,
      harnesses: input.harnesses && Object.keys(input.harnesses).length > 0 ? input.harnesses : null,
    },
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
  /** The user's ordered selection of which harnesses gate validation — tier 2
   *  of the engine's harness resolution chain. Order is the user's (cheap
   *  gates first) and has to be stored, because `harnesses` is a map with no
   *  order to inherit. `null`/empty = no selection, which resolves exactly as
   *  it does today. */
  validation_gates?: string[] | null;
  prepare_command?: string | null;
  extra_writable_paths?: string[] | null;
  conflict_policy?: string;
  feature_lifecycle?: string;
  default_agent_kind?: string | null;
  default_model?: string | null;
  default_effort?: EffortLevel | null;
  default_loop_iterations?: number | null;
  default_max_budget_usd?: number | null;
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
  const existing = await getProposedStrategy(projectId);
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
      // Dropped rather than written when the selection is empty: the backend
      // stores the bare `harnesses` map in that case, so a project that never
      // ticks a gate keeps writing byte-identical rows.
      validation_gates:
        input.validation_gates !== undefined
          ? (input.validation_gates && input.validation_gates.length > 0
              ? input.validation_gates
              : null)
          : (baseWs?.validation_gates ?? null),
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
    // Omitted by every caller that doesn't own the Strategy tab, so it has to
    // be carried across from the stored record — otherwise saving any other
    // setting would silently drop the project's default effort.
    default_effort:
      input.default_effort !== undefined
        ? input.default_effort
        : (existing?.default_effort ?? null),
    default_loop_iterations:
      input.default_loop_iterations !== undefined
        ? input.default_loop_iterations
        : (existing?.default_loop_iterations ?? null),
    default_max_budget_usd:
      input.default_max_budget_usd !== undefined
        ? input.default_max_budget_usd
        : (existing?.default_max_budget_usd ?? null),
    artifact_subdir:
      input.artifact_subdir ?? existing?.artifact_subdir ?? "artifacts/",
    commit_artifacts:
      input.commit_artifacts ?? existing?.commit_artifacts ?? false,
  };

  await invoke("save_project_settings", { projectId, settings: merged });
}
