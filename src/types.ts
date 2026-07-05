// Post-pivot types. Legacy supervisor/thread types were removed as part of
// the R7 cleanup; see AGENT_INTEGRATION.md §1 for the surviving surface.

/** Release channel baked into the binary at build time. */
export type ReleaseChannel = 'stable' | 'nightly';

/** Application version + channel, surfaced by the `get_app_version` IPC. */
export interface AppVersion {
  version: string;
  channel: ReleaseChannel;
}

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
  | { kind: 'create-project' }
  | { kind: 'project-settings' }
  | { kind: 'workflows' }
  | { kind: 'workflow-editor'; workflowId: string | null }
  | { kind: 'providers' }
  | { kind: 'settings' }
  | { kind: 'remote-inbox' };

/** Laptop-side mirror of one remote run (docs/REMOTE_EXECUTION_PLAN.md
 *  M6.1/M6.2), keyed by `(machine_id, run_id)` — mirrors
 *  `demeteo_core::ports::remote_run_mirror::RemoteRunMirror`. */
export interface RemoteRunMirror {
  machine_id: string;
  run_id: string;
  project_id: string | null;
  title: string;
  status: string;
  error: string | null;
  feature_id: string | null;
  pr_url: string | null;
  pushed_branch: string | null;
  last_offset: number;
  created_at: number;
  updated_at: number;
  last_notified_status: string | null;
}

/** One entry in a remote run's append-only event log (M3.3/M6.4) —
 *  mirrors `demeteo_core::ports::run_events::RunEvent`. */
export interface RunEvent {
  offset: number;
  run_id: string;
  kind: string;
  payload_json: string | null;
  created_at: number;
}

// ── Create-Project Wizard ──────────────────────────────────────────────
//
// The create-project wizard is a routed `AppView` that walks the user
// through **exactly seven** one-decision-per-screen steps and then
// auto-launches `wf-starter-standard` against the freshly-created repo
// (see `src-tauri/src/domain/bootstrap.rs` + `commands/create_project.rs`).
//
// The variant order is fixed; the React step components import the
// matching kebab-case slugs from `BootstrapStep`.

/** The seven wizard steps, in canonical order. Locked — do not reorder
 *  or insert variants; the React shell, the Rust `BootstrapStep`
 *  enum, and the spec all share this contract. */
export const BootstrapStep = {
  Name: 'name',
  Provider: 'provider',
  Group: 'group',
  Machine: 'machine',
  Agent: 'agent',
  Model: 'model',
  Description: 'description',
} as const;
export type BootstrapStep = typeof BootstrapStep[keyof typeof BootstrapStep];

/** All seven steps in display order — single source of truth for the
 *  progress indicator. The wizard's `goBack` rewind MUST be based on
 *  `state.history` (not a raw index into this array) so auto-progressed
 *  steps cannot be silently re-entered; see `WizardShell`. */
export const STEP_ORDER: ReadonlyArray<BootstrapStep> = [
  BootstrapStep.Name,
  BootstrapStep.Provider,
  BootstrapStep.Group,
  BootstrapStep.Machine,
  BootstrapStep.Agent,
  BootstrapStep.Model,
  BootstrapStep.Description,
];

/** The wizard's state, as it lives in React memory between IPC calls.
 *  Mirrors the Rust `BootstrapState` struct. The Rust side also stores
 *  the canonical history (including auto-progressed entries) so the
 *  frontend cannot lose its place on `goBack`. */
export interface BootstrapState {
  step: BootstrapStep;
  history: BootstrapStep[];
}

/** Step-specific payload sent to `submit_create_project_step`. Mirrors
 *  the Rust `CreateProjectStepPayload` discriminated union exactly
 *  (kebab-case `step` tag). The variant order matters: the wizard UI
 *  must emit the variant matching the current `BootstrapState.step`,
 *  otherwise the Rust command rejects the call with a Validation error. */
export type CreateProjectStepPayload =
  | { step: 'name'; value: string }
  | { step: 'provider'; providerId: string; kind: string }
  | { step: 'group'; namespaceId: string; kind: string; name: string }
  | { step: 'machine'; kind: 'local' | 'remote'; machineId: string | null }
  | { step: 'agent'; kind: string }
  | { step: 'model'; model: string }
  | {
      step: 'commit';
      title: string;
      description: string;
      visibility: 'private' | 'public';
      name: string;
      providerId: string;
      providerKind: string;
      providerHost: string;
      namespaceId: string;
      namespaceKind: string;
      namespaceName: string;
      machineKind: 'local' | 'remote';
      machineId: string | null;
      agentKind: string;
      model: string;
    };

/** Result of `submit_create_project_step` from the Rust command. The
 *  frontend matches on `kind` to decide whether to stay in the wizard
 *  (continue → render the returned state's current step) or navigate
 *  to the launched feature's Detail view. */
export type BootstrapOutcome =
  | { kind: 'continue'; state: BootstrapState }
  | { kind: 'launched'; feature: LaunchedFeature };

/** Compact view of a successfully-launched feature, returned to the
 *  wizard so it can navigate to the Detail view. Mirrors the Rust
 *  `LaunchedFeature`. Field names are snake_case to match the IPC
 *  payload (no `rename_all = "camelCase"` on the Rust side). The
 *  `created_repo` field carries the same `CreatedRepo` shape that
 *  `src/lib/createProjectWizard.ts` exposes — see that module. */
export interface LaunchedFeature {
  feature_id: string;
  feature_title: string;
  project_id: string;
  created_repo: {
    full_name: string;
    default_branch: string;
    clone_url: string;
  };
}

// NOTE: `CreatedRepo` and `ProviderNamespace` are declared and
// exported from `src/lib/createProjectWizard.ts` (alongside their
// Tauri command wrappers). They are NOT re-declared here to avoid a
// duplicate-interface compile error — wizard components import them
// directly from the lib.

export type ConfigOptionValue = {
  value: string;
  name: string;
  description?: string;
  /**
   * Best-effort signal that the chosen model can accept attached images.
   * Mirrors the Rust `ConfigOptionValue.supports_images` field on the
   * model probe response. When the backend cannot confirm vision support
   * (fallback list miss or unknown dynamically-probed model) this is
   * `false` so the UI can show a soft warning rather than silently drop
   * an attached image.
   */
  supports_images?: boolean;
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
  on_failure?: string | null;
  max_iterations?: number | null;
  verifier?: VerifierConfig | null;
  /** Role-based permission posture. See {@link StepCapability}. */
  capability?: StepCapability | null;
  /** Opt the step into web search / fetch (e.g. research consulting live docs). */
  allow_network?: boolean;
  /** Opt a non-shell capability into the shell (e.g. an Artifacts step that wants `git log`). */
  allow_shell?: boolean;
  /**
   * Blast-radius classification for `gate` steps (docs/REMOTE_EXECUTION_PLAN.md M5.1).
   * `"dangerous"` (merge-to-default / push-protected / deploy / delete) parks under an
   * unattended remote run instead of auto-approving; anything else is the `safe` default.
   */
  gate_class?: 'safe' | 'dangerous' | null;
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
  /**
   * Prompt-cache read tokens billed at the discounted rate for
   * the active step. Populated from the agent's `Usage` /
   * `TurnComplete` events (opencode / hermes / claude-code). Not
   * persisted to SQLite in the Tier-1 cut — surfaced via the live
   * `step_progress` event only.
   */
  cache_read_input_tokens?: number | null;
  /**
   * Prompt-cache creation tokens (priced ABOVE base input — a
   * one-time write cost). Populated alongside `cache_read_input_tokens`.
   */
  cache_creation_input_tokens?: number | null;
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
  provider_id: string;
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

/** Global agent-turn timeout configuration. Mirrors the Rust `AgentTimeouts`
 * struct. All values are in seconds.
 *
 * - `fast_timeout_s` (300): when no event arrives for this many seconds
 *   after at least one event has been seen, the turn is aborted with
 *   "Agent blocked: no output for Ns".
 * - `normal_timeout_s` (600): when no event has ever arrived for this many
 *   seconds, the turn is aborted with "Agent response timed out".
 * - `wall_cap_s` (1800): absolute wall-clock cap per turn. */
export interface AgentTimeouts {
  fast_timeout_s: number;
  normal_timeout_s: number;
  wall_cap_s: number;
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
   * Optional shell command run inside each subtask worktree before the
   * verifier's harness command (e.g. `npm ci`, `cargo fetch`, `prisma
   * generate`). Runs after dependency-cache dirs from the primary
   * checkout are symlinked in and after write permissions are restored.
   * `null`/unset skips this step.
   */
  prepare_command?: string | null;
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
