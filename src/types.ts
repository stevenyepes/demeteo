// Post-pivot types. Legacy supervisor/thread types were removed as part of
// the R7 cleanup; see AGENT_INTEGRATION.md §1 for the surviving surface.

export interface Project {
  id: string;
  name: string;
  status: string;
  repos: number;
  nodes: number;
  spend: number;
  tokens: number;
  compute_type?: string;
  remote_host?: string | null;
}

export interface Provider {
  id: string;
  type: string;
  name: string;
  host: string;
  pat: string;
  username: string;
  avatarUrl: string;
}

export interface Machine {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: string;
  key_path?: string | null;
  agents?: string | null;
  use_login_shell?: boolean | null;
  setup_commands?: string | null;
}

export interface EditorContext {
  machineId: string;
  worktreePath: string;
  branch: string;
  defaultBranch: string;
  initialFile?: string;
}

export interface WorkflowSummary {
  id: string;
  name: string;
  description: string;
  version: number;
}

export type AppView =
  | { kind: 'empty-state' }
  | { kind: 'home' }
  | { kind: 'detail'; featureId: string; featureTitle: string; gateStepExecutionId?: string | null }
  | { kind: 'editor'; editorContext: EditorContext; featureId: string; featureTitle: string }
  | { kind: 'new-project' }
  | { kind: 'project-settings' }
  | { kind: 'workflows' }
  | { kind: 'workflow-editor'; workflowId: string | null }
  | { kind: 'providers' }
  | { kind: 'settings' };

export type ConfigOptionValue = {
  value: string;
  name: string;
  description?: string;
};

export interface ConfigOption {
  id: string;
  name: string;
  description?: string;
  category?: string;
  type: string;
  currentValue: string;
  options: ConfigOptionValue[];
}

export interface Workflow {
  id: string;
  name: string;
  description: string;
  is_starter: boolean;
  created_at: number;
  updated_at: number;
  schedule?: WorkflowSchedule | null;
}

/**
 * What a step is allowed to do. Drives the agent permission profile (tool
 * policy) and the chmod write-scope fence on the Rust side. When omitted,
 * the backend infers a safe default (`artifacts` for ordinary agent steps,
 * `implement` for parallel / unconstrained-write steps).
 * - `read_only`: inspect/review only — no writes, no shell, no network.
 * - `artifacts`: read + write only under `artifacts/` — no shell, no network.
 * - `verify`: read + run build/test/lint + write only under `artifacts/`.
 * - `implement`: full read/write/shell within the worktree.
 */
export type StepCapability = 'read_only' | 'artifacts' | 'verify' | 'implement';

export type StepConfig = {
  id: string;
  kind: 'agent' | 'parallel' | 'gate' | string;
  title: string;
  agent_kind?: string | null;
  model?: string | null;
  prompt_template?: string | null;
  artifact_mode: 'full' | 'summary_only' | 'none' | string;
  on_failure?: string | null;
  max_iterations?: number | null;
  verifier?: VerifierConfig | null;
  /** Role-based permission posture. See {@link StepCapability}. */
  capability?: StepCapability | null;
  /** Opt the step into web search / fetch (e.g. research consulting live docs). */
  allow_network?: boolean;
  /** Opt a non-shell capability into the shell (e.g. an Artifacts step that wants `git log`). */
  allow_shell?: boolean;
};

export interface WorkflowWithSteps extends Workflow {
  steps: StepConfig[];
  version: number;
  version_id: string;
}

/**
 * A project-scoped harness (coding agent) + model override for a workflow or a
 * single step. Mirrors `ProjectWorkflowOverride` in Rust (migrations V14/V15).
 * `step_id == null` is the workflow-level override (applies to all steps); a
 * non-null `step_id` targets one step. `null` on agent_kind/model means
 * "inherit" for that field; a record absent from the list inherits both.
 */
export interface WorkflowOverride {
  project_id: string;
  workflow_id: string;
  step_id?: string | null;
  agent_kind?: string | null;
  model?: string | null;
}

export interface StepExecution {
  id: string;
  feature_id: string;
  step_id: string;
  step_index: number;
  step_kind: string;
  status: 'pending' | 'running' | 'awaiting_gate' | 'completed' | 'failed' | 'skipped' | 'interrupted' | string;
  cost_usd?: number | null;
  tokens?: number | null;
  wall_clock_secs?: number | null;
  artifact_path?: string | null;
  artifact_paths: string[];
  error_message?: string | null;
  iteration_count?: number;
  created_at: number;
  updated_at: number;
}

export interface GateDecision {
  id: string;
  step_execution_id: string;
  decision?: 'approve' | 'redirect' | 'cancel' | string | null;
  feedback?: string | null;
  created_at: number;
}

export interface Feature {
  id: string;
  project_id: string;
  workflow_id?: string;
  title: string;
  status: string;
  total_cost: number;
  tokens?: number | null;
  duration: string;
  created_at: number;
  agent_kind?: string | null;
  model?: string | null;
  /** URL of the published PR/MR, if any. Set by the `MrPublisher`. */
  mr_url?: string | null;
  /**
   * State of the PR/MR on the provider: `none | draft | open | merged | closed`.
   * `none` → no MR has been published. `open` is the typical "review pending"
   * state. The UI shows this as a badge on the feature detail.
   */
  mr_state?: string | null;
  /**
   * Per-feature override for `ProjectSettings.commit_artifacts`.
   * `null`/`undefined` → inherit the project default.
   * `true` → agent reports (`research-report.md`, `critic-review.md`, …)
   * are committed into the feature branch.
   * `false` → reports stay in demeteo's local store + UI only.
   * Set from the StartFeatureModal advanced section.
   */
  commit_artifacts?: boolean | null;
}

export type MrState = 'none' | 'draft' | 'open' | 'merged' | 'closed';

export type NotificationKind =
  | 'mr_merged'
  | 'gate_pending'
  | 'step_failed'
  | 'feature_completed'
  | 'merge_conflict'
  | 'retry_budget_exhausted';

/** Mirrors the Rust `Notification` struct on the `notifications`
 *  table. `feature_url` is a relative deep link; the bell decides
 *  how to route it. */
export interface Notification {
  id: string;
  project_id: string;
  feature_id: string;
  kind: NotificationKind;
  message: string;
  feature_url?: string | null;
  read: boolean;
  created_at: number;
}

/** Wire shape of `DomainEvent::MrMerged` as emitted by the
 *  Tauri notification adapter. The bell listens for this to
 *  refetch + toast without a full poll. */
export interface MrMergedEvent {
  feature_id: string;
  project_id: string;
  feature_title: string;
  mr_url: string;
}

/** Wire shape of `DomainEvent::RetryBudgetExhausted` — fired by
 *  the orchestrator when a step's `on_failure` retry chain runs
 *  out of attempts. The user must intervene; the agent gave up.
 *  Drives the toast in `NotificationBell`. */
export interface RetryBudgetExhaustedEvent {
  feature_id: string;
  step_id: string;
  target_id: string;
  attempt: number;
  max: number;
  reason: string;
}

/** Return shape for `feature_sync` and `feature_resolve_sync_conflicts`. */
export type SyncOutcomeView =
  | {
      status: 'ok';
      merge_commit_sha: string;
      changed: boolean;
    }
  | {
      status: 'conflict';
      conflict_files: ConflictFile[];
      raw_error: string;
    }
  | {
      status: 'resolved';
      merge_commit_sha: string;
      revalidated_step_id: string | null;
    }
  | {
      status: 'resolution_failed';
      reason: string;
      conflict_files: ConflictFile[];
    };

export interface ConflictFile {
  path: string;
  /** "both-modified" | "added-by-them" | "added-by-us" | "deleted-by-them" | "deleted-by-us". */
  kind: string;
}

export interface Repository {
  id: string;
  repo_path: string;
}

export interface VerifierConfig {
  agent_kind?: string | null;
  instructions: string;
  harness_name?: string | null;
  verdict_key?: string;
}

export interface WorkflowSchedule {
  cron: string;
  title_template: string;
  project_id: string;
  next_run_at?: number | null;
}

export type MemoryType =
  | 'convention'
  | 'lesson'
  | 'decision'
  | 'preference'
  | 'fact';

export interface ProjectMemoryEntry {
  id: string;
  project_id: string;
  key: string;
  value: string;
  source: 'agent' | 'human';
  confidence: number;
  memory_type: MemoryType | null;
  statement: string | null;
  embedding: number[] | null;
  embedding_model: string | null;
  last_used_at: number | null;
  use_count: number;
  created_at: number;
  updated_at: number;
}

/** Global config for the background memory agent. Mirrors the Rust
 * `MemoryAgentConfig`. The API key is never returned to the UI — only
 * `has_api_key` indicates whether one is stored. */
export interface MemoryAgentConfig {
  enabled: boolean;
  chat_endpoint: string;
  chat_model: string;
  embed_endpoint: string;
  embed_model: string;
  has_api_key: boolean;
  top_k: number;
  min_confidence: number;
}

export interface MemoryAgentTestResult {
  chat_ok: boolean;
  embed_ok: boolean;
  embed_dims: number | null;
  error: string | null;
}

/**
 * Discriminated-union mirror of the Rust `AppError` enum.
 * Stable across releases — the `kind` field is the IPC contract;
 * do not rename variants without coordinating with the backend.
 */
export type AppErrorKind =
  | 'not_found'
  | 'validation'
  | 'conflict'
  | 'provider'
  | 'transport'
  | 'database'
  | 'agent'
  | 'internal';

export interface AppError {
  kind: AppErrorKind;
  message: string;
}

export interface WorktreeStrategy {
  default_branch: string;
  branch_prefix: string;
  test_command: string | null;
  build_command: string | null;
  coverage_command: string | null;
  conventions_file: string | null;
  pr_template: string | null;
  harnesses?: Record<string, string> | null;
  /**
   * Project-level writability exceptions for the chmod scope fence.
   * Repo-relative paths the agent may write to even when its step
   * capability (`read_only`, `artifacts`, `verify`) would otherwise
   * fence them. Common uses: `target/` for `cargo test`,
   * `node_modules/` for `npm test`, `.venv/` for `pytest`.
   * Backend normalises entries (rejects absolute paths and `..`) and
   * merges into the per-step writable set.
   */
  extra_writable_paths?: string[] | null;
}

export interface ProjectSettingsData {
  project_id: string;
  worktree_strategy: WorktreeStrategy;
  conflict_policy: string;
  feature_lifecycle: string;
  default_agent_kind?: string | null;
  default_model?: string | null;
  default_loop_iterations?: number | null;
  artifact_subdir?: string;
  commit_artifacts?: boolean;
}
