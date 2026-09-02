// Post-pivot types. Legacy supervisor/thread types were removed as part of
// the R7 cleanup; see AGENT_INTEGRATION.md §1 for the surviving surface.

import type { AttachedFile } from './lib/attachments';
import type { EffortLevel } from './lib/effortLevels';

/** The reasoning-effort ladder. Declared (and drift-tested) in
 *  `lib/effortLevels.ts`; re-exported here so IPC types can name it without
 *  every consumer reaching into `lib/`. */
export type { EffortLevel };

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
  /** "Away" notification webhook (docs/REMOTE_EXECUTION.md M6.3) — any URL
   *  accepting `{"text": "..."}`, which is the shape Slack incoming webhooks
   *  and ntfy.sh have in common. Injected into the runner's systemd unit at
   *  install time; blank disables it. */
  notify_webhook_url?: string | null;
}

export interface EditorContext {
  machineId: string;
  worktreePath: string;
  branch: string;
  defaultBranch: string;
  initialFile?: string;
  /** The pair the Changes tab diffs, when the caller has a narrower one in mind
   *  than "this branch against its base". A sync resolution is reviewed as
   *  `head_before..merge_commit_sha`: the first-parent form reads correctly and
   *  goes silently wrong the moment the resolver adds a follow-up commit, and
   *  nothing afterwards can recover the real base. Omitted = the branch pair,
   *  as before. */
  baseRef?: string;
  headRef?: string;
  /** Which sidebar tab opens. Omitted = 'files'. */
  initialTab?: 'files' | 'changes';
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
  | {
      kind: 'detail';
      featureId: string;
      featureTitle: string;
      gateStepExecutionId?: string | null;
      /**
       * Which step the inspector is showing, held here so it survives
       * back/forward and a deep link (UI redesign plan §3.5).
       *
       * One key serves two surfaces, so the id is either a `step_executions`
       * id (what the timeline selects) or a graph node id (what the canvas
       * selects); `src/lib/inspectorTarget.ts` resolves both against the run's
       * steps. An id that resolves to neither is routine rather than a fault —
       * a reload can replace every execution row under a still-valid link — and
       * degrades to a named empty state there, never an error.
       *
       * Optional *and* nullable, and the two are not interchangeable: absent
       * and `null` are the inputs the seeding policy in
       * `components/FeatureDetail/useStepSelection.ts` (see `SelectionIntent`)
       * tells apart. Normalising one to the other here — a `?? null`, a
       * required field, a route that fills the gap — collapses that policy from
       * a distance where nothing fails loudly.
       */
      selectedStepId?: string | null;
    }
  | { kind: 'editor'; editorContext: EditorContext; featureId?: string; featureTitle?: string }
  | { kind: 'new-project' }
  | { kind: 'create-project' }
  | { kind: 'project-settings' }
  | { kind: 'code-review' }
  | { kind: 'workflows' }
  | { kind: 'workflow-editor'; workflowId: string | null }
  /** One Discovery's workspace. The title rides along so the header can name
   *  it before `discovery_get` answers, exactly as `detail` carries
   *  `featureTitle`. */
  | { kind: 'discovery'; discoveryId: string; discoveryTitle: string }
  /** One project's Ask workspace — the thread list, transcript and canvas
   *  live inside `AskThreadView` itself rather than a ProjectHome card grid,
   *  so this carries only the project, not a thread id. */
  | { kind: 'ask'; projectId: string }
  | { kind: 'providers' }
  | { kind: 'settings' }
  | { kind: 'remote-inbox' }
  | { kind: 'terminals' };

/** Laptop-side mirror of one remote run (docs/REMOTE_EXECUTION.md
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

/** A single feature-start sub-step, streamed while a feature is
 *  `bootstrapping` — mirrors `DomainEvent::BootstrapProgress`. Delivered as
 *  a Tauri `bootstrap_progress` event (local) or a `bootstrap_progress`
 *  {@link RunEvent} in the durable log (remote/detached). `phase` is a stable
 *  id the UI orders by; `label` is rendered verbatim; `status` drives the
 *  status dot. */
export interface BootstrapProgressPayload {
  feature_id?: string;
  phase: string;
  label: string;
  status: 'running' | 'completed' | 'failed' | 'skipped' | string;
  detail?: string | null;
}

/** Canonical display order for the bootstrap stepper. Phases not listed
 *  (e.g. a future addition) sort after these, in first-seen order. Mirrors
 *  the `bootstrap_phase` vocabulary in `demeteo-core` plus the runner's
 *  clone phases. */
export const BOOTSTRAP_PHASE_ORDER: ReadonlyArray<string> = [
  'cloning',
  'detecting_strategy',
  'preparing',
  'connecting',
  'verifying_repo',
  'preparing_context',
  'syncing_origin',
  'creating_branch',
  'harness_preflight',
  'registering',
  'starting_pipeline',
];

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
 *  (kebab-case `step` tag, **snake_case** fields — the enum has no
 *  `rename_all_fields`, so serde matches the field names literally, the
 *  same convention as the sibling `CreateProjectConfig` IPC struct).
 *  The variant order matters: the wizard UI must emit the variant
 *  matching the current `BootstrapState.step`, otherwise the Rust
 *  command rejects the call with a Validation error. */
export type CreateProjectStepPayload =
  | { step: 'name'; value: string }
  | { step: 'provider'; provider_id: string; kind: string }
  | { step: 'group'; namespace_id: string; kind: string; name: string }
  | { step: 'machine'; kind: 'local' | 'remote'; machine_id: string | null }
  | { step: 'agent'; kind: string }
  | { step: 'model'; model: string; effort?: EffortLevel | null }
  | {
      step: 'commit';
      title: string;
      description: string;
      visibility: 'private' | 'public';
      name: string;
      provider_id: string;
      provider_kind: string;
      provider_host: string;
      namespace_id: string;
      namespace_kind: string;
      namespace_name: string;
      machine_kind: 'local' | 'remote';
      machine_id: string | null;
      agent_kind: string;
      model: string;
      /** Seeds `ProjectSettings.default_effort`. Omitted = no project
       *  default, which resolves to the engine default (`high`). */
      effort?: EffortLevel | null;
    };

export type DescriptionStepPatch = {
  step: 'commit';
  title?: string;
  description?: string;
  visibility?: 'private' | 'public';
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
 * `implement` for sequence / unconstrained-write steps).
 * - `read_only`: inspect/review only — no writes, no shell, no network.
 * - `artifacts`: read + write only under `artifacts/` — no shell, no network.
 * - `verify`: read + run build/test/lint + write only under `artifacts/`.
 * - `implement`: full read/write/shell within the worktree.
 */
export type StepCapability = 'read_only' | 'artifacts' | 'verify' | 'implement';

export type StepConfig = {
  id: string;
  /**
   * `parallel` is the superseded name for `sequence`. Its concurrent fan-out
   * was removed; steps still carrying the old kind are executed sequentially.
   * Kept in the union so existing saved workflows still type-check.
   */
  kind: 'agent' | 'sequence' | 'parallel' | 'gate' | string;
  title: string;
  agent_kind?: string | null;
  model?: string | null;
  /** Reasoning effort for this step. Unset = inherit (project default, then
   *  the engine default `high`). A peer of `model`, not a property of it. */
  effort?: EffortLevel | null;
  /**
   * `sequence` steps only: the earlier step whose `task-list` artifact holds
   * the ordered task list to execute. Unset falls back to the step planning
   * the work itself.
   */
  task_list_from?: string | null;
  /**
   * Declared outputs. The editor does not author these — they live in the
   * workflow JSON — but it reads their names to offer the `task_list_from`
   * sources the backend lint will accept, and they round-trip untouched
   * through a save.
   */
  artifacts?: { name: string }[] | null;
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
   * Blast-radius classification for `gate` steps (docs/REMOTE_EXECUTION.md M5.1).
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
  effort?: EffortLevel | null;
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

/**
 * One row of a step's per-attempt history (`step_attempts`, P1.8) — mirror of
 * the Rust `StepAttempt`. Each dispatch of a step (an `on_failure` redirect, an
 * environmental in-place retry, a manual retry) opens a fresh attempt instead
 * of overwriting the step row, so the node drill-down panel (P2.3) can show the
 * class/cost/duration/applied-rule of every try. Fetched via `step_attempts_list`.
 */
export interface StepAttempt {
  step_execution_id: string;
  /** 1-based, dense per step execution. */
  attempt_no: number;
  /** `running | completed | failed | cancelled | interrupted | redirected`. */
  status: string;
  /** This attempt's own delta, not the step's running total. */
  cost_usd?: number | null;
  tokens?: number | null;
  wall_clock_ms?: number | null;
  /** `environment | verdict | agent_failure | non_retryable`; null for non-failures. */
  error_class?: string | null;
  failure_fingerprint?: string | null;
  /** The retry-policy rule that answered this failure, `<class>.<strategy>` (P1.10). */
  applied_rule?: string | null;
  workspace_fingerprint?: string | null;
  idempotency_key?: string | null;
  started_at: number;
  ended_at?: number | null;
}

/**
 * One task of a `sequence` node's list, merged for the drill-down accordion
 * (P2.5) — mirror of the Rust `SequenceTaskView`. `landed` (the committed
 * Decision-13 prefix) is the load-bearing distinction the accordion renders.
 * Fetched via `sequence_tasks_list`.
 */
export interface SequenceTaskView {
  id: string;
  title: string;
  /** `landed | running | completed | failed | interrupted | skipped | pending`.
   *  `landed` wins over the subtask row — a committed task is done. */
  status: string;
  /** True when the task's commit is on the feature branch (won't re-run). */
  landed: boolean;
  /** Which decomposition cycle planned this task: 0 for the original list,
   *  incrementing once per rework cycle (a downstream verdict sending the
   *  run back to the step that produces the list). */
  cycle: number;
  /** True for tasks planned by an earlier cycle — shown for context, not
   *  part of what the current cycle runs. Always `landed`. */
  prior_cycle: boolean;
  cost_usd?: number | null;
  tokens?: number | null;
  error_message?: string | null;
}

/** A `sequence` node's whole task list (`SequenceState`). `planned` is false
 *  before the node resolves a plan, distinct from an empty plan. */
export interface SequenceState {
  planned: boolean;
  tasks: SequenceTaskView[];
}

/** Mirror of the Rust `TaskPlan`/`PlannedTask`/`PlanCycle`
 *  (`crates/demeteo-core/src/domain/sequence/tasks.rs:20-153`) — the
 *  `task-list.json` artifact a `sequence` node's decomposition step writes.
 *  The artifact is agent-written, not compiler-checked, so only `id`/
 *  `title`/`description` on a task and `tasks` on the plan may be assumed
 *  present; everything else mirrors a Rust `#[serde(default)]` field.
 *
 *  `kind`/`cycle`/`history` are optional for a sharper reason than serde
 *  defaults: no artifact on disk carries them at all. The shape the producer
 *  is shown is tasks-only (`task_list_json_shape_example`), and the sequence
 *  step assigns cycle bookkeeping into its own DB plan cache without ever
 *  writing it back to the file. A reader of this artifact must therefore
 *  supply its own label, not trust one to be there. */
export type PlanKind = 'greenfield' | 'rework';

export interface PlannedTask {
  id: string;
  title: string;
  description: string;
  files?: string[];
  test_command?: string | null;
  acceptance?: string[];
  blocked_by?: string[];
  retry_note?: string | null;
}

export interface PlanCycle {
  tasks: PlannedTask[];
  cycle?: number;
  kind?: PlanKind;
}

export interface TaskPlan {
  tasks: PlannedTask[];
  kind?: PlanKind;
  cycle?: number;
  history?: PlanCycle[];
  notes?: string | null;
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
  /** Persisted prompt body typed at launch (migration V27). `''` for runs
   *  started before the column existed. */
  description?: string;
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
  /**
   * What the project's validation gates said at this run's **base commit**
   * (decision 44, `features.harness_baseline_json`, migration V37). Mirrors the
   * Rust `Feature::harness_baseline`, so it arrives on `feature_get` for a
   * local run and on the hydrated shadow row for a detached one.
   *
   * `null`/`undefined` means **no baseline was measured**, which is emphatically
   * not "everything was green" — see {@link HarnessBaseline}.
   */
  harness_baseline?: HarnessBaseline | null;
  /** Where this run's branch was cut from (`features.origin_json`, V41).
   *  Absent on every row written before V41, which cut from the default
   *  branch. */
  origin?: FeatureOrigin | null;
  /** The branch the run is measured against. `null` = the project's default
   *  branch. Not interchangeable with {@link Feature.origin}: a run started
   *  from a pull-request head reviews against the branch it merges into, not
   *  against the snapshot it began at. */
  diff_base_branch?: string | null;
  /** The branch the run actually works on, written down at cut time.
   *  `null` on pre-V41 rows, which re-derive `{branch_prefix}{id}`. */
  resolved_branch?: string | null;
}

/** Mirrors the Rust `FeatureOrigin` (`domain/feature_origin.rs`), which serde
 *  tags internally on `kind` with snake_case arm names. Declared twice with no
 *  codegen between them, so the discriminants here are the wire contract.
 *
 *  `ref` carries a `fetch_spec` the Rust side re-validates into a `Refspec` on
 *  the way in — TypeScript can state the shape but not the constraint, so
 *  nothing here may treat an accepted string as a safe one. */
export type FeatureOrigin =
  | { kind: 'default_branch' }
  | { kind: 'branch'; base: string }
  | { kind: 'ref'; fetch_spec: string; label: string };

/** Which producer measured one gate — mirrors the Rust `BaselineProducer`.
 *  Recorded per gate, not per record, because a partial re-measurement merges
 *  into an existing record. */
export type BaselineProducer = 'node' | 'fallback';

/** Why a gate was red at the base **because the machine could not run it** —
 *  mirrors the Rust `BaselineEnvironmentFault`. Its presence is what separates
 *  a pre-existing *code* defect (subtracted from the verdict) from a gate that
 *  reached no verdict at all (terminal, with remediation). */
export interface BaselineEnvironmentFault {
  reason: string;
  /** The concrete provisioning step. May be empty — the classifier is not
   *  obliged to know one. */
  remediation: string;
}

/** One gate's measurement at the base commit — mirrors the Rust
 *  `HarnessBaselineRun`. */
export interface HarnessBaselineRun {
  name: string;
  /** The command as the user authored it (not the `2>&1` wrapper). */
  command: string;
  exit_ok: boolean;
  /** Normalized failure fingerprint; empty when the gate was green. */
  fingerprint?: string;
  /** `ArtifactStore` reference to the merged output — never the output. */
  output_ref?: string | null;
  /** Set only when the classifier said the gate could not run here. */
  environment?: BaselineEnvironmentFault | null;
  /** The test identifiers this gate's **red** measurement named, read out of
   *  its own output (rung 3 of the granularity ladder).
   *
   *  Absent means *no reading was obtained* — a green gate, an extractor that
   *  answered nothing, or a record written before the field existed — never
   *  "the runner named no failing test". */
  failing_tests?: string[] | null;
  /** Unix **seconds** at which this gate was measured. */
  measured_at: number;
  producer: BaselineProducer;
}

/** Everything measured at one base commit, for one feature — mirrors the Rust
 *  `HarnessBaseline`.
 *
 *  **Absent is not green.** A gate missing from `harnesses` was never measured,
 *  and rendering that as a pass inverts the whole subtraction decision 44
 *  exists to make: a genuine regression would read as pre-existing. Every
 *  consumer here answers "not measured" rather than filling the silence. */
export interface HarnessBaseline {
  /** The commit the measurement was taken against. A record describing another
   *  commit is not evidence about this run. */
  base_sha: string;
  /** The gates measured, in the order they ran (cheap gates first). */
  harnesses?: HarnessBaselineRun[];
}

export type MrState = 'none' | 'draft' | 'open' | 'merged' | 'closed';

export type NotificationKind =
  | 'mr_merged'
  | 'gate_pending'
  | 'step_failed'
  | 'feature_completed'
  | 'merge_conflict'
  | 'retry_budget_exhausted'
  | 'environment_not_ready';

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

/** Wire shape of `DomainEvent::EnvironmentNotReady` — fired when a
 *  harness failure is triaged (C6) as an environment problem (missing
 *  system library / toolchain / service, permission or network fault)
 *  the coding agent cannot fix. Fired immediately, before the retry
 *  budget is spent; `reason` carries the full remediation + reproduce
 *  line. Drives the toast in `NotificationBell`. */
export interface EnvironmentNotReadyEvent {
  feature_id: string;
  step_id: string;
  reason: string;
}

/**
 * Where a sync stopped short of a merge verdict. Mirrors
 * `crate::domain::sync_failure::SyncBlockedStage`; every spelling here is the
 * wire form, pinned variant by variant on the Rust side by
 * `the_serialized_shape_is_the_wire_contract`. A stage this union does not
 * carry is silent on arrival — the banner's per-stage sentence resolves to
 * `undefined`, which React renders as nothing.
 */
export type SyncBlockedStage =
  | 'fetch'
  | 'base_ref_missing'
  | 'worktree_provision'
  | 'merge'
  | 'push'
  | 'verify'
  | 'feature_diverged'
  | 'repo_context'
  | 'held_resolution'
  | 'turn_in_flight';

/**
 * How far a feature branch has drifted from the base a sync would merge.
 *
 * Both counts are nullable because "we could not measure it" and "there is
 * nothing to merge" are different facts, and only the second one means the
 * branch is current. Rendering a null as `0` is how a branch nobody could look
 * at ends up labelled up to date.
 */
export interface BranchDivergence {
  behind: number | null;
  ahead: number | null;
}

/** Return shape for `feature_drift`. */
export interface FeatureDrift {
  divergence: BranchDivergence;
  /** The ref the counts were taken against, e.g. `origin/main`. */
  base_ref: string;
  /** `false` when the fetch was skipped or failed, so the counts are as of
   *  whenever that ref last moved rather than as of now. */
  fetched: boolean;
  checked_at: number;
}

/**
 * What may be done about a feature branch that has diverged from
 * `origin/<feature>` — the question the two counts cannot answer, because two
 * commits ahead of a branch that already carries their changes and two commits
 * ahead of one that does not are the same pair of numbers.
 *
 * The wire form of `domain::upstream_feature::DivergenceMove`, read off `git
 * cherry`. `refuse` is the non-answer — a partial rewrite, or a read that could
 * not be made — and it is a value rather than an absence because "we looked and
 * cannot say" is the fact that withholds the reset.
 */
export type DivergenceMove = 'merge_origin' | 'reset_onto_origin' | 'refuse';

/** The half of `DivergenceMove` that names something a person can press. */
export type DivergenceReconcile = Exclude<DivergenceMove, 'refuse'>;

/** Return shape for `feature_divergence`. */
export interface FeatureDivergence {
  /** Commits this checkout has that `origin/<feature>` does not. */
  ahead: number;
  /** Commits `origin/<feature>` has that this checkout does not. */
  behind: number;
  next_move: DivergenceMove;
}

/** Return shape for `feature_sync` and `feature_resolve_sync_conflicts`. */
export type SyncOutcomeView =
  | {
      status: 'ok';
      /** `null` when the tip the merge left could not be read. */
      merge_commit_sha: string | null;
      changed: boolean;
    }
  | {
      status: 'conflict';
      conflict_files: ConflictFile[];
      raw_error: string;
    }
  | {
      status: 'blocked';
      stage: SyncBlockedStage;
      raw_error: string;
    }
  | {
      status: 'resolved';
      merge_commit_sha: string;
    }
  | {
      status: 'resolution_failed';
      reason: string;
      conflict_files: ConflictFile[];
    };

/**
 * Who would resolve a conflict on this feature if the banner's picker is left
 * alone, as `feature_sync_resolver` answers it. Read rather than derived: the
 * chain behind it puts the project's conflict-resolver setting above the
 * harness the run was launched with, so the feature's own row is the wrong
 * answer for any project that has set one.
 */
export interface SyncResolverView {
  agent_kind: string;
  model: string | null;
  effort: EffortLevel;
}

export interface ConflictFile {
  path: string;
  /** "both-modified" | "added-by-them" | "added-by-us" | "deleted-by-them" | "deleted-by-us". */
  kind: string;
}

/**
 * The state a feature's sync is in. Mirrors
 * `crate::domain::sync_session::SyncSessionStatus`; every spelling here is the
 * wire form.
 */
export type SyncSessionState =
  | 'syncing'
  | 'up_to_date'
  | 'merged'
  | 'blocked'
  | 'conflicted'
  | 'resolving'
  | 'resolved'
  | 'resolution_failed'
  | 'aborted';

/**
 * One feature's live sync, as `sync_session_get` answers it — reconciled
 * against the working tree on the way out, so a `conflicted` session here is
 * one git still agrees with.
 */
export interface SyncSessionView {
  feature_id: string;
  machine_id: string;
  repo_dir: string;
  feature_branch: string;
  base_branch: string;
  status: SyncSessionState;
  worktree_path: string | null;
  /** The feature branch's tip before the merge — the base a review diff of the
   *  resolution has to be computed from. */
  head_before: string | null;
  merge_commit_sha: string | null;
  conflict_files: ConflictFile[];
  /** git's own stderr, verbatim. */
  raw_error: string | null;
  /** Where a `blocked` sync stopped (migration V46), or `null` on any other
   *  status and on a row written before that migration. `'push'` is the one
   *  stage that has already committed the merge onto the feature branch, so it
   *  is the one a retry would strand — read it, never guess it from
   *  `raw_error`. */
  blocked_stage: SyncBlockedStage | null;
  /** When the resolution reached origin, or `null` while it is only on the
   *  branch. `status === 'resolved'` with a `null` here is a resolution waiting
   *  for a look, not a finished sync — no probe of the working tree can answer
   *  this, which is why it is a field and not a tenth `status`. Migration
   *  V45. */
  pushed_at: number | null;
  attempts: number;
  created_at: number;
  updated_at: number;
  /** Whether this sync is the user's to abort or re-resolve, rather than one a
   *  live run is already driving. Computed by
   *  `domain::sync_session::user_may_intervene` — a `resolving` session, or any
   *  session on a feature whose run is still going, belongs to that turn: abort
   *  would delete the worktree an agent is writing in and resolve would put a
   *  second agent in the same tree. Never re-derive this from `status` here; the
   *  window where the row still reads `conflicted` while the step owns it is
   *  exactly what the backend flag closes. */
  user_may_intervene: boolean;
}

export interface Repository {
  id: string;
  repo_path: string;
  provider_id: string;
}

export interface VerifierConfig {
  agent_kind?: string | null;
  instructions: string;
  /** Ordered gates: each runs separately, in this order, and all must pass.
   *  Empty/absent falls through to the project's selected validation gates and
   *  then to its `test_command` (HB5). */
  harness_names?: string[] | null;
  /** @deprecated The pre-HB5 single-harness spelling. Still parsed by the
   *  backend for workflows authored against it; never written any more. */
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
   * The user's **ordered selection of which harnesses gate validation** —
   * tier 2 of the engine's harness resolution chain, beaten only by a step
   * that names its own gates. It exists because every shipped starter
   * declares none, so without it the `harnesses` map is config nothing ever
   * reads. Ordered because the map has no order to inherit and the order is
   * the user's: cheap gates first, lint before integration.
   * `null`/empty = no selection, which resolves as it does today.
   */
  validation_gates?: string[] | null;
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
  /** Project-wide default reasoning effort. `null` = no project default,
   *  which resolves to the engine default (`high`) at run time. */
  default_effort?: EffortLevel | null;
  /** The workflow a new feature in this project starts on. `null`/absent = the
   *  project has not chosen one, which is not the same as having none: the
   *  launch modal then falls back explicitly rather than taking whatever
   *  `workflow_list` returned first. See migration V40. */
  default_workflow_id?: string | null;
  default_loop_iterations?: number | null;
  /** Project-wide default per-turn dollar budget passed to the agent as
   *  `--max-budget-usd`. `null` = no project default, which resolves to the
   *  engine default ($20) at run time. Overridable per run. */
  default_max_budget_usd?: number | null;
  artifact_subdir?: string;
  commit_artifacts?: boolean;
  /** The command a reviewing step starts from, as the user wrote it. Carried
   *  into the prompt verbatim — Demeteo neither validates it against a harness
   *  nor wraps it in review vocabulary of its own. `null`/absent (and the empty
   *  string, which a cleared input writes) = the project names none, and the
   *  step is left to review in its own way. Migration V42. */
  review_entrypoint?: string | null;
  /** The harness a merge-conflict resolution runs under, outranking the run's
   *  own launch pin for that turn alone. `null`/absent = no opinion, which
   *  falls through to the run and then to `default_agent_kind`. Migration
   *  V44. */
  sync_resolver_agent_kind?: string | null;
  /** The model for that turn, inherited independently of the harness. */
  sync_resolver_model?: string | null;
  /** The reasoning effort for that turn, clamped per harness at spawn. */
  sync_resolver_effort?: EffortLevel | null;
  /** Whether a resolved sync waits for a human before it is published.
   *  `null`/absent = no opinion, which holds only when somebody is in a
   *  position to look at it and publishes otherwise. `false` opts out. It
   *  cannot impose review on a run that still owns its branch — see
   *  `domain::sync_session::publish_policy`. Migration V45. */
  sync_review_before_push?: boolean | null;
}

export interface SessionInfo {
  session_id: string;
  machine_id: string;
  created_at: number;
  title: string | null;
  /** Friendly machine name (`Machine.name`), present on start / list /
   *  agent events. Absent on lifecycle events where the frontend already
   *  knows the label. */
  machine_name?: string | null;
  /** Coding-agent kind currently detected in the session (`"claude-code"`,
   *  `"opencode"`, …), or null/absent for a plain shell. */
  agent?: string | null;
}

/** A selectable repository checkout returned by the terminal-worktree API. */
export interface TerminalWorktree {
  path: string;
  branch: string | null;
  isLocked: boolean;
}

/** Where a session may open inside one repository. `mainBranch` is what the
 *  main checkout is currently sitting on — a session opened there inherits it,
 *  and nothing else in this payload reveals which branch that is. `null` for a
 *  detached or unreadable checkout, which is a reason to say nothing rather
 *  than to name a branch. */
export interface TerminalLocations {
  mainBranch: string | null;
  worktrees: TerminalWorktree[];
}

/** The only caller-controlled inputs for creating a terminal worktree. */
export interface CreateTerminalWorktreeRequest {
  projectId: string;
  repositoryId: string;
  branch: string;
  /** The branch to cut from. The backend fetches it from origin first; `null`
   *  leaves the start point at whatever the main checkout is sitting on. */
  baseBranch: string | null;
  worktreeName: string;
}

/** A branch offered as a base, and where it exists. `hasRemote` is what says a
 *  fetch can refresh it — a `false` there means the base is only as current as
 *  the local copy. */
export interface TerminalBranchOption {
  name: string;
  hasLocal: boolean;
  hasRemote: boolean;
}

/** Base candidates plus the branch this project integrates into, which the
 *  picker preselects. */
export interface TerminalBranchOptions {
  defaultBranch: string;
  branches: TerminalBranchOption[];
}

/** A created worktree and the ref its branch actually started at —
 *  `origin/<base>` when the fetch reached origin, a bare `<base>` when it did
 *  not. Reported so the UI can say which, rather than promising the first. */
export interface CreatedTerminalWorktree {
  worktree: TerminalWorktree;
  baseRef: string;
}

/**
 * What the agent in a terminal is doing right now, layered on top of
 * presence (`agentKind`). Sourced live from the backend activity sweep
 * (`terminal-session-activity`). `null` means no activity signal — a
 * plain shell, or an agent we can't read yet (spec `TERMINAL_ACTIVITY`
 * §2): the row shows the agent badge and no activity mark.
 */
export type TerminalActivity =
  | 'working'
  | 'awaiting_input'
  | 'awaiting_approval'
  | null;

export interface TerminalTabDescriptor {
  sessionId: string | null;
  tabId: string;
  machineId: string;
  machineLabel: string;
  projectId?: string;
  repoPath?: string;
  workBranch?: string | null;
  title: string;
  phase: 'connecting' | 'running' | 'disconnected' | 'closed' | 'error';
  createdAt: number;
  /** Coding-agent kind running in this tab (`"claude-code"`, `"opencode"`,
   *  …), or null for a plain shell. Seeded from the launch command and kept
   *  live for local tabs by the backend foreground detector. */
  agentKind?: string | null;
  /** Live activity of the agent in this tab (working / waiting / needs a
   *  decision), or null/absent when no signal is available. Driven by the
   *  backend `terminal-session-activity` sweep. */
  activity?: TerminalActivity;
}

export interface TerminalPanelState {
  tabs: TerminalTabDescriptor[];
  activeTabId: string | null;
}

// ── Discovery (docs/PRD_DISCOVERY.md) ─────────────────────────────────────
//
// Hand-mirrored from the Rust serde shapes, which is the only mechanism this
// repo has: `domain/models/discovery.rs`, `domain/models/ticket.rs`,
// `domain/ticket_graph.rs`, `application/discovery/mod.rs` and
// `application/tickets/mod.rs`. Every id newtype is `#[serde(transparent)]`,
// so a `DiscoveryId` or `TicketId` arrives as a bare string.

/** Mirrors `DiscoveryStatus`. */
export type DiscoveryStatus = 'open' | 'closed';

/** Mirrors `MessageRole`. There is no system role — what the interviewer is
 *  told is assembled per turn, so a stored copy would describe a world that
 *  has moved on. */
export type MessageRole = 'user' | 'assistant';

/** Mirrors `Discovery`. */
export interface Discovery {
  id: string;
  project_id: string;
  title: string;
  status: DiscoveryStatus;
  machine_id: string;
  agent_kind: string;
  model: string | null;
  effort: EffortLevel | null;
  resume_session_id: string | null;
  worktree_path: string | null;
  /** What the user handed the interviewer. Owned by the Discovery rather than
   *  by a turn, so the composer's chip row survives the turn that added it and
   *  every later turn is prompted with the same set. */
  attachments: AttachedFile[];
  total_cost: number;
  tokens: number;
  created_at: number;
  updated_at: number;
}

/**
 * Mirrors `DiscoverySummary`, which `#[serde(flatten)]`s the `Discovery` — so
 * the row's own fields arrive alongside the two the card needs and the row
 * does not carry.
 *
 * `progress` is the counter `discovery_board` derives, from the same pass over
 * the same rows: a second, SQL-shaped opinion would disagree with the card the
 * user then opens.
 */
export interface DiscoverySummary extends Discovery {
  message_count: number;
  progress: TicketProgress;
}

/** Mirrors `QuestionOption`. */
export interface QuestionOption {
  id: string;
  label: string;
  description: string;
}

/** Mirrors `DiscoveryQuestion`. `recommended` names a `QuestionOption.id`;
 *  `null` is a real answer, not a missing one. */
export interface DiscoveryQuestion {
  header: string;
  text: string;
  options: QuestionOption[];
  recommended: string | null;
}

/** Mirrors `DiscoveryMessage`. `cost_usd`/`tokens` are `null` on a user turn
 *  and on an assistant turn whose harness reported no spend — distinct from
 *  `0`, which is a measurement. */
export interface DiscoveryMessage {
  id: string;
  discovery_id: string;
  role: MessageRole;
  content: string;
  cost_usd: number | null;
  tokens: number | null;
  /** What the turn did, collected while it streamed. `null` on a user message
   *  and on any turn stored before V49 — absent, never "it touched nothing". */
  activity: TurnActivity | null;
  created_at: number;
}

/** Mirrors `TurnActivity` (V49). `commands` is a bounded sample of what `ran`
 *  counts, stored as the agent issued them; the name a reader sees is derived
 *  from it by `lib/discoveryActivity.ts`, so the live turn and the settled one
 *  cannot name the same command differently. */
export interface TurnActivity {
  reads: number;
  edits: number;
  writes: number;
  ran: number;
  commands: readonly string[];
}

/** Mirrors `DiscoveryMessageView`, which `#[serde(flatten)]`s a
 *  `DiscoveryMessage` and the `InterviewTurn` derived from its text — so both
 *  halves arrive on one object. Which question is *open* is derived one level
 *  further out, by the reader: the last one with no user message after it. */
export interface DiscoveryMessageView extends DiscoveryMessage {
  prose: string;
  question: DiscoveryQuestion | null;
  nothing_left_to_settle: boolean;
  question_error: string | null;
}

/** Mirrors `DiscoveryDetail`. */
export interface DiscoveryDetail {
  discovery: Discovery;
  messages: DiscoveryMessageView[];
  /** The decompose pass waiting to be reviewed, or `null`. Stored against the
   *  Discovery, so a pass the user navigated away from is still theirs when
   *  they come back — including after a restart. */
  pending_proposal: DecomposeProposal | null;
  /** A turn or a pass is running *right now*. Known only within the process
   *  that started it, so `false` after a restart is the truth rather than a
   *  gap: nothing survived it to still be running. */
  turn_running: boolean;
}

/** Mirrors `TicketState` — the whole stored vocabulary. Everything a screen
 *  shows about a ticket beyond these three is derived on read. */
export type TicketState = 'unstarted' | 'started' | 'dropped';

/** Mirrors `Ticket`. `attachments` stage here and are committed to the
 *  Feature when the ticket starts, so a ticket that never starts never writes
 *  an attachment row. */
export interface Ticket {
  id: string;
  discovery_id: string;
  /** The number a user says out loud. Assigned once and never reissued, so a
   *  list index would rename every ticket after a deletion. */
  seq: number;
  title: string;
  description: string;
  acceptance: string[];
  files: string[];
  blocked_by: string[];
  test_command: string | null;
  workflow_id: string | null;
  agent_kind: string | null;
  model: string | null;
  effort: EffortLevel | null;
  attachments: AttachedFile[];
  state: TicketState;
  drop_reason: string | null;
  force_start_reason: string | null;
  force_started_at: number | null;
  feature_id: string | null;
  created_at: number;
  updated_at: number;
}

/** Mirrors `TicketLane`. A closed-unmerged ticket lands in `dropped`: it
 *  satisfies its dependents yet nothing of it reached the base branch, so
 *  neither `in_flight` nor `landed` would be true of it. */
export type TicketLane = 'blocked' | 'ready' | 'in_flight' | 'landed' | 'dropped';

/** Mirrors `BlockerReason`. `unknown` is a dangling edge — drift rather than
 *  a plan — and is reported apart from `outstanding` so a surface can say
 *  *unknown prerequisite* rather than *waiting*. */
export type BlockerReason = 'outstanding' | 'unknown';

/** Mirrors `Blocker`. */
export interface Blocker {
  id: string;
  reason: BlockerReason;
}

/** Mirrors `TicketStanding`. */
export interface TicketStanding {
  id: string;
  lane: TicketLane;
  startable: boolean;
  blockers: Blocker[];
}

/** Mirrors `TicketProgress`. `live` is every lane but `dropped` — the
 *  denominator §9.2 specifies, since a dropped ticket is not work
 *  outstanding. */
export interface TicketProgress {
  blocked: number;
  ready: number;
  in_flight: number;
  landed: number;
  dropped: number;
  live: number;
}

/** Mirrors `TicketFeatureView` — what a started ticket's current attempt
 *  contributes to a card. */
export interface TicketFeatureView {
  id: string;
  status: string;
  mr_state: string | null;
  mr_url: string | null;
}

/** Mirrors `TicketView`: the row, its derived position, and the forge state
 *  the position was derived from, which travel together so the graph and the
 *  board cannot disagree. */
export interface TicketView {
  ticket: Ticket;
  standing: TicketStanding;
  feature: TicketFeatureView | null;
}

/** Mirrors `DiscoveryBoard`. `tickets` arrive in `Ticket.seq` order. */
export interface DiscoveryBoard {
  tickets: TicketView[];
  progress: TicketProgress;
}

// ── Decomposition (docs/PRD_DISCOVERY.md §5) ──────────────────────────────
//
// Every id in this half of the wire is **proposal-space**: what the agent
// authored, not what a row is stored under. `discovery_apply_decomposition`
// names the changes it accepts by those ids and mints the stored ones itself,
// so nothing here may be treated as a `Ticket.id`.

/** Mirrors `PlannedTicket` — one ticket as the decomposition wrote it, before
 *  a workflow *name* is a workflow id. Carried out and handed straight back:
 *  the proposal is not persisted anywhere. */
export interface PlannedTicket {
  id: string;
  title: string;
  description: string;
  acceptance: string[];
  files: string[];
  test_command: string | null;
  blocked_by: string[];
  /** A workflow name, as the prompt listed it. The agent has no ids. */
  workflow: string | null;
  agent: string | null;
  model: string | null;
  effort: string | null;
  /** Why this ticket is in *this* pass, addressed to the reviewer. */
  why: string | null;
}

/** Mirrors `ChangeKind` — the modal's first three groups. `Locked` is not one
 *  of them: a locked ticket is listed, never proposed. */
export type ChangeKind = 'added' | 'revised' | 'removed';

/** Mirrors `FieldChange`. Both sides arrive as text because the modal renders
 *  two lines, and nine field types formatted per call site would be nine
 *  formattings. */
export interface FieldChange {
  field: string;
  was: string;
  now: string;
}

/** Mirrors `ProposedChange` — one reviewable row, and one checkbox. */
export interface ProposedChange {
  id: string;
  kind: ChangeKind;
  /** `null` for an addition: `seq` is assigned at apply and never reissued,
   *  so a proposal has no number to show yet. */
  seq: number | null;
  title: string;
  why: string | null;
  workflow_name: string | null;
  agent_kind: string | null;
  blocked_by: string[];
  /** Empty except on a revision. */
  fields: FieldChange[];
}

/** Mirrors `LockedTicket` — a started ticket, listed so the user can see what
 *  the pass worked around. */
export interface LockedTicket {
  id: string;
  seq: number;
  title: string;
  lane: TicketLane | null;
}

/** Mirrors `ImmutableChange`. */
export type ImmutableChange = 'revised' | 'removed';

/** Mirrors `ImmutableViolation` — a started ticket the pass tried to touch,
 *  reported per ticket rather than as one sentence. */
export interface ImmutableViolation {
  id: string;
  change: ImmutableChange;
  reason: string;
}

/** Mirrors `DecomposeProposal`. */
export interface DecomposeProposal {
  discovery_id: string;
  /** The discovery held no tickets before this pass — the `First pass`
   *  eyebrow. Derived, never counted. */
  first_pass: boolean;
  /** The plan verbatim. `discovery_apply_decomposition` takes it back
   *  unchanged. */
  tickets: PlannedTicket[];
  changes: ProposedChange[];
  locked: LockedTicket[];
  /** Every refusal the pass was re-asked over, oldest first — including the
   *  ones it then fixed, which is what the validation bar reports. */
  refused: string[];
  /** Set when the last attempt was refused too, so nothing here can be
   *  applied. */
  refusal: string | null;
  violations: ImmutableViolation[];
  cost_usd: number;
  tokens: number;
}

/** Mirrors `DecomposeApply`. */
export interface DecomposeApply {
  discovery_id: string;
  tickets: PlannedTicket[];
  /** The `ProposedChange.id`s left checked. A change absent from this list
   *  leaves its stored row alone. */
  accept: string[];
}

/**
 * Mirrors `TicketEdit`.
 *
 * **Every key is required, and none of them means "leave this one alone".**
 * Rust reads an absent key and an explicit `null` identically, so a partial
 * payload would turn *clear the model* into *keep the model* with nothing on
 * screen to say so. The drawer holds the whole ticket and saves it whole.
 */
export interface TicketEdit {
  title: string;
  description: string;
  acceptance: string[];
  files: string[];
  blocked_by: string[];
  test_command: string | null;
  workflow_id: string | null;
  agent_kind: string | null;
  model: string | null;
  effort: EffortLevel | null;
}

/** Mirrors `AskStatus`. Ask never surfaces `closed` yet — no close/reopen
 *  command exists — but the wire vocabulary already carries it. */
export type AskStatus = 'open' | 'closed';

/** Mirrors `AskThread`. */
export interface AskThread {
  id: string;
  project_id: string;
  title: string;
  status: AskStatus;
  agent_kind: string;
  model: string | null;
  effort: EffortLevel | null;
  machine_id: string;
  /** Reserved for `ask-turn-loop`; this ticket never populates it. */
  worktree_path: string | null;
  /** Reserved for `ask-turn-loop`; this ticket never populates it. */
  session_id: string | null;
  turn_count: number;
  cost_usd: number;
  tokens: number;
  /** Whether the thread's agent may reach the network. */
  network: boolean;
  created_at: number;
  updated_at: number;
}

/** Mirrors `CanvasPathVerdict`. Whether a path a canvas node cited resolves
 *  against the tree checked at `AskMessage.checked_commit_sha`. */
export interface CanvasPathVerdict {
  node_id: string;
  path: string;
  resolved: boolean;
}

/** Mirrors `AskMessage`. */
export interface AskMessage {
  id: string;
  thread_id: string;
  role: MessageRole;
  text: string;
  cost_usd: number | null;
  tokens: number | null;
  turn_activity: TurnActivity | null;
  canvas_paths: CanvasPathVerdict[] | null;
  checked_commit_sha: string | null;
  created_at: number;
}

/** Mirrors `AskMessageView`, which `#[serde(flatten)]`s an `AskMessage` and
 *  the `AskTurn` derived from its text — the same shape `DiscoveryMessageView`
 *  takes for the same reason: a turn and what it drew can never disagree
 *  about each other if neither is stored. */
export interface AskMessageView extends AskMessage {
  prose: string;
  canvas: AskCanvas | null;
  canvas_error: string | null;
}

/** Mirrors `AskThreadDetail`. */
export interface AskThreadDetail {
  thread: AskThread;
  messages: AskMessageView[];
}

/** Mirrors `NodeRole`. */
export type NodeRole = 'orchestration' | 'boundary' | 'agent' | 'needs_human';

/** Mirrors `CanvasKind`. Not branched on by the renderer — see Constraints. */
export type CanvasKind = 'architecture' | 'journey' | 'dataflow';

/** Mirrors `EdgeKind`. */
export type EdgeKind = 'hands_off' | 'goes_back';

/** Mirrors `CanvasNode`. A non-null `path` here is not itself a resolved
 *  signal — the renderer credits a path only when its
 *  `CanvasPathVerdict.resolved` says so. `path: null` is a third answer, not
 *  the same as `resolved: false`: it means the node named a person or a
 *  concept and never claimed a file, so it renders as an ordinary card. See
 *  `NodePathState` in `AskCanvasNode.tsx`. */
export interface CanvasNode {
  id: string;
  title: string;
  role: NodeRole;
  path: string | null;
  stage: number;
  lane: number;
}

/** Mirrors `CanvasEdge`. */
export interface CanvasEdge {
  from: string;
  to: string;
  kind: EdgeKind;
}

/** Mirrors `AskCanvas`. */
export interface AskCanvas {
  kind: CanvasKind;
  title: string;
  stages: string[];
  lanes: string[];
  nodes: CanvasNode[];
  edges: CanvasEdge[];
}

/** Mirrors `PinnedCanvasEntry`. `title` and `pinned_at` are null for an entry
 *  whose snapshot body could not be read or parsed — the row still opens on
 *  `path`, so a corrupt pin costs its own label and nothing else. */
export interface PinnedCanvasEntry {
  path: string;
  title: string | null;
  pinned_at: number | null;
}

/** Mirrors `NodeResolution`. */
export type NodeResolution =
  | { kind: 'editor'; machine_id: string; worktree_path: string; branch: string; default_branch: string; path: string }
  | { kind: 'moved'; checked_commit_sha: string };
