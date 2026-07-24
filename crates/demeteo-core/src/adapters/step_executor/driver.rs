use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::watch;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::gate_waiter::GateWaiter;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::{EffortLevel, StepConfig};
use crate::domain::prompt_context::PromptContext;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::attachment_store::AttachmentStore;
use crate::ports::db::{AppSettingsRepository, FeatureRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::NotificationPort;
use crate::ports::pricing::PricingTable;

// ── Sub-modules ────────────────────────────────────────────────────────────────
//
// Each submodule owns one slice of the driver's responsibility. The top-level
// `driver.rs` is now just the struct + a thin `run()` delegate — see
// `run_loop/mod.rs` for the loop's decomposition into dispatch / outcome /
// attempt / cleanup.

pub(crate) mod failure;
pub(crate) mod publish;
pub(crate) mod resolution;
pub(crate) mod run_loop;
pub(crate) mod signals;
pub(crate) mod status;
pub(crate) mod verifier;
pub(crate) mod watchdog;

pub(crate) use super::driver_registry::DriverRegistry;

/// The default `on_failure` retry-loop budget when neither the run override
/// (`Feature::loop_iterations`), the project setting
/// (`ProjectSettings::default_loop_iterations`), nor the step's own
/// `max_iterations` is set.
pub(crate) const DEFAULT_LOOP_ITERATIONS: u32 = 3;

/// Feedback captured when a step fails and the loop redirects back to an
/// earlier step. Injected into the retried step's prompt as
/// `{{retry_feedback}}` / `{{iteration}}` / `{{max_iterations}}` so the
/// retry isn't blind. Held in-memory for the lifetime of a single run.
#[derive(Clone)]
pub(crate) struct RetryContext {
    /// Raw failure / verifier reason from the step that triggered the loop.
    pub feedback: String,
    /// 1-based attempt number we're now starting.
    pub iteration: u32,
    /// Effective max iterations for this loop.
    pub max: u32,
    /// Failing test identifiers from a structured verdict (empty for
    /// plain failures). Lets the retried step's prompt name the exact
    /// tests to fix and the sequence step target the owning tasks.
    pub failing_tests: Vec<String>,
    /// Repo-relative files a structured verdict implicated (empty for
    /// plain failures). The sequence step re-runs only the tasks
    /// whose ownership intersects these.
    pub implicated_files: Vec<String>,
    /// Step id of the step whose failure opened this loop iteration.
    /// The feedback stays alive for *every* step between the redirect
    /// target and this step, and is cleared only when this step finally
    /// completes — so e.g. a re-run of `s-validate` still knows what it
    /// failed on last time instead of re-checking blind. Empty string
    /// means "clear after the next completed step" (legacy behavior,
    /// used by synthesized per-subtask contexts).
    pub failing_step_id: String,
}

/// Holds all shared state for a single feature execution run.
pub(crate) struct ExecutionDriver {
    // Repository / service Arcs
    pub features: Arc<dyn FeatureRepository>,
    pub gates: Arc<dyn crate::ports::db::GateRepository>,
    pub projects: Arc<dyn crate::ports::db::ProjectRepository>,
    pub signals: Arc<dyn crate::ports::memory_signals::MemorySignalsPort>,
    pub notif: Arc<dyn NotificationPort>,
    /// Notification persistence port. The driver uses this to
    /// write a row to the `notifications` table when a user-visible
    /// event is emitted from inside a step (e.g. retry budget
    /// exhausted). Mirrors the same `SqliteAdapter` instance as
    /// `features` / `gates`; no separate I/O.
    pub notifications: Arc<dyn crate::ports::db::NotificationRepository>,
    pub registry: Arc<AgentRegistry>,
    pub agent_exec: Arc<dyn AgentExecutionPort>,
    pub exec: Arc<dyn ExecutionPort>,
    pub artifacts: Arc<dyn ArtifactStore>,
    /// Per-feature user attachment store. Read by `spawn.rs` to copy
    /// attachments into the per-step worktree before each agent turn
    /// so they sit inside the `external_directory: deny` fence. See
    /// `step_executor::artifacts::materialize_user_attachments_to_worktree`.
    pub attachments: Arc<dyn AttachmentStore>,
    /// Shared `app_settings` KV repository. Used by every agent-turn call
    /// site to resolve the effective timeouts via
    /// [`crate::application::timeouts::resolve_effective`]. Owned by the
    /// executor; cloned from `DagStepExecutor.app_settings` at driver
    /// construction so changes mid-run are visible to subsequent turns.
    pub app_settings: Arc<dyn AppSettingsRepository>,
    pub git_ops: GitOpsHelper,
    pub merge_executor: Arc<dyn MergeExecutor>,
    /// Per-task run telemetry for `sequence` steps. The task loop opens a
    /// `subtask_runs` row when a task's agent spawns and closes it when the
    /// task commits or fails, so the dashboard's live "nodes" count and the
    /// post-hoc audit of which tasks ran are both real.
    pub subtask_runs: Arc<dyn crate::ports::db::SubtaskRunRepository>,
    /// Opens the PR once the last step finishes. The same publisher the
    /// Publish button has always used — an HTTP call to the provider's API,
    /// never the `gh` CLI. `None` under the headless runner, which opens its
    /// own PR at the end of `run.rs` with a memory-only PAT instead (it holds
    /// no keyring credential this could resolve).
    pub mr_publisher: Option<Arc<dyn crate::ports::mr_publisher::MrPublisher>>,
    pub gate_waiters: Arc<Mutex<HashMap<String, Arc<GateWaiter>>>>,
    pub driver_registry: Arc<DriverRegistry>,

    /// Model → USD pricing. Threaded through every `stream_agent_turn`
    /// call so the [`UsageAccumulator`](crate::domain::usage::UsageAccumulator)
    /// can compute a fallback cost when the agent's wire format omits it.
    pub pricing: Arc<dyn PricingTable>,

    // Feature identity
    pub f_id: FeatureId,
    pub f_id_str: String,

    // Pre-computed setup
    pub machine_id_opt: Option<String>,
    pub target_dir: String,
    pub branch_name: String,
    pub base_ctx: PromptContext,
    pub steps: Vec<StepConfig>,

    // Mutable execution state
    pub step_index: usize,
    pub start_time: Instant,
    pub cancel_watch: watch::Receiver<bool>,

    /// Repo-relative folder where agents write their reports.
    /// Snapshotted at feature-start time from the project settings
    /// (and the Feature row's per-feature override). The driver
    /// passes this to every `commit_worktree_changes` call so the
    /// orchestrator can include or exclude the folder from the
    /// commit depending on `commit_artifacts`. See migration V12
    /// and `commit_worktree_changes` in
    /// `artifacts/declared.rs`.
    pub artifact_subdir: String,

    /// Whether to include `artifact_subdir` in
    /// `commit_worktree_changes`. `true` → reports land in the PR.
    /// `false` → reports stay in demeteo's `FsArtifactStore` only.
    /// Resolved at feature-start time as
    /// `features.commit_artifacts ?? settings.commit_artifacts`.
    pub commit_artifacts: bool,

    /// Project-level writability exceptions for the chmod scope fence.
    /// Snapshotted from `ProjectSettings.worktree_strategy.extra_writable_paths`
    /// at feature start so changes to project settings mid-run don't
    /// silently widen the fence. Passed to every step's
    /// `derive_writable_paths_for_scope` call alongside the
    /// capability-derived scope.
    pub extra_writable_paths: Vec<String>,

    // --- Agent/model resolution inputs (snapshotted at feature start) ---
    /// Feature-wide run override of the agent kind (the run modal's
    /// "apply to all"). Beats the workflow step but loses to a per-step
    /// override. `None` = not set.
    pub feature_agent_kind: Option<String>,
    /// Feature-wide run override of the model. Same precedence as
    /// `feature_agent_kind`.
    pub feature_model: Option<String>,
    /// Feature-wide run override of the reasoning effort. Same precedence as
    /// `feature_model`. `None` = inherit (never "default").
    pub feature_effort: Option<EffortLevel>,
    /// Per-step agent/model/effort overrides chosen at launch (highest
    /// precedence).
    pub step_overrides: Vec<crate::domain::models::StepOverride>,
    /// Project default agent kind (`ProjectSettings::default_agent_kind`).
    pub default_agent_kind: Option<String>,
    /// Project default model (`ProjectSettings::default_model`).
    pub default_model: Option<String>,
    /// Project default effort (`ProjectSettings::default_effort`). Last tier
    /// before the built-in [`EffortLevel::DEFAULT`].
    pub default_effort: Option<EffortLevel>,

    // --- Loop budget inputs ---
    /// Per-run override of the loop budget (`Feature::loop_iterations`).
    pub loop_iterations_override: Option<u32>,
    /// Project default loop budget (`ProjectSettings::default_loop_iterations`).
    pub project_default_loop_iterations: Option<u32>,
    /// Per-run override of the per-turn dollar budget (`Feature::max_budget_usd`).
    pub max_budget_usd_override: Option<f64>,
    /// Project default per-turn dollar budget
    /// (`ProjectSettings::default_max_budget_usd`).
    pub project_default_max_budget_usd: Option<f64>,

    /// Set when a step fails and the loop redirects to an earlier step;
    /// consumed by the next step's prompt build, then cleared.
    pub retry_ctx: Option<RetryContext>,

    // --- Context-window watchdog state (token optimization, Tier 1) ---
    /// Resolved model name for the *current* step's primary agent.
    /// Used by the watchdog to look up the model's context-window
    /// budget via [`PricingTable::context_window`]. Updated as the
    /// driver walks steps so model changes mid-run take effect.
    pub current_model: Option<String>,

    /// Model's known context-window size in tokens (input + output).
    /// `None` when the model is unknown to the pricing table or for
    /// local / free models — watchdog skips the threshold check in
    /// that case (legacy behavior).
    pub context_budget_tokens: Option<u64>,

    /// Set by `compact_or_reset` after the watchdog kills the
    /// session for exceeding budget. The next step's
    /// `spawn_agent_session` will spawn a fresh session and inject
    /// the `session_resume_summary` so the agent has a one-shot
    /// recap of what the prior session concluded.
    pub session_dirty: bool,

    /// Injected at the top of the next prompt when the watchdog
    /// resets the session. Built from the prior session's last
    /// completed step's artifact + key feature context. Empty
    /// string on the first step (no recap needed).
    pub session_resume_summary: String,

    /// Cumulative input+output tokens billed against the
    /// feature-wide agent session. Updated by the agent step's
    /// post-turn path; mirrored from the registry session's
    /// `cumulative_tokens()` so the watchdog can compare against
    /// `context_budget_tokens` after each step.
    pub session_cumulative_tokens: u64,

    /// Last-seen cache-read and cache-creation token counts from the
    /// current step's `TurnOutcome`. Surfaced on the `StepProgress`
    /// notification so the UI can render a live "saved $X.XX by
    /// cache" chip while the step is running.
    pub last_cache_read: Option<u64>,
    pub last_cache_creation: Option<u64>,

    /// The current step's agent-session registry key (see
    /// `ExecutionDriver::agent_session_key`). Recomputed by
    /// `refresh_watchdog_budget` at the top of each step so
    /// `maybe_watchdog_reset` — which runs right after the step
    /// finishes — targets the exact session `spawn_agent_session`
    /// just used, rather than a bare feature id that no longer
    /// identifies a single session once sessions are fingerprint
    /// (permission profile + model) scoped.
    pub current_session_key: String,
}

impl ExecutionDriver {
    /// Run the full execution loop. The body lives in
    /// [`run_loop::run`](run_loop::run) — this is the thin shell that
    /// the executor's spawn-tail calls into. Kept here (rather than
    /// fully delegated to a free function) so the type's public API
    /// stays symmetric: `driver.run().await`.
    pub(crate) async fn run(self) {
        self::run_loop::run(self).await;
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/driver.rs"]
mod resolution_tests;

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/driver_watchdog.rs"]
mod watchdog_tests;
