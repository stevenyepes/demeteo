//! `RunSpec` — the payload that drives a headless run
//! (docs/REMOTE_EXECUTION_PLAN.md M1.2). Carries everything the runner
//! needs to bootstrap a project, ingest a workflow, and start a feature,
//! without depending on any state already present in the runner's own
//! (fresh) database — the laptop composes this from its own DB state and
//! ships it over. Reused as-is by the control-RPC `submit_run` payload
//! (M3) once the laptop⇄runner channel exists.

use serde::{Deserialize, Serialize};

/// Git provider push access needed to clone/push the repo (R5/§6.2). For
/// M1, the PAT rides in an env var on the runner host and is bridged into
/// the existing keyring-backed `GitOpsHelper` credential path — M4
/// replaces this with proper in-memory-only injection over the control
/// channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpecProvider {
    /// `"github"` | `"gitlab"`.
    pub kind: String,
    /// e.g. `"github.com"`, or a GitHub Enterprise / self-hosted GitLab host.
    pub host: String,
}

/// Per-run hard caps on token cost and wall-clock time (docs/REMOTE_EXECUTION_PLAN.md
/// M5.2, docs/REMOTE_EXECUTION.md §5). Exceeding either cap cancels the run
/// as `over-budget` — unattended never auto-approves more spend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunBudget {
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub max_wall_clock_secs: Option<u64>,
}

/// A run submitted to a headless runner: enough to bootstrap a project,
/// ingest a workflow, and start a feature from a clean database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    /// Laptop-chosen feature id the runner must reuse in `feature_start`,
    /// so the eager shadow Feature the laptop inserts at submit time and
    /// the runner's own Feature row share one id — the run is navigable
    /// on the laptop from second zero and hydration (C4.2) updates the
    /// placeholder in place. `None` (an old laptop) lets the runner
    /// generate its own id as before; an old runner ignores this field.
    #[serde(default)]
    pub feature_id: Option<String>,
    /// Short human label — becomes the feature title and (via the
    /// project's `WorktreeStrategy`) part of the branch name.
    pub title: String,
    /// The rich prompt body rendered into `{{feature_description}}`.
    pub description: String,
    /// Git provider hosting `repo_path`.
    pub provider: RunSpecProvider,
    /// `owner/repo` (no scheme, no host) — same shape
    /// `GitOpsHelper::clone_repository` expects.
    pub repo_path: String,
    /// The workflow to run, embedded inline (R3: results ride git, but
    /// the workflow definition itself rides the run spec, not a
    /// pre-existing DB row) — parsed the same way
    /// `commands::workflows::workflow_create` parses a user-authored
    /// workflow: `{ "name", "description", "steps": [...] }`.
    pub workflow_json: serde_json::Value,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    /// Per-run override of the `on_failure` retry-loop budget. `None`
    /// inherits the engine default.
    pub loop_iterations: Option<u32>,
    /// R6/R7 (M5.1): when true, the runner auto-approves gates
    /// classified `safe` (`StepConfig::gate_class`) and parks anything
    /// classified `dangerous` instead of waiting indefinitely for a
    /// human. The per-command permission/intercept layer and worktree
    /// fence are unaffected either way — unattended relaxes gates only.
    #[serde(default)]
    pub unattended: bool,
    /// M5.2 hard caps. `None` = no cap on that dimension.
    #[serde(default)]
    pub budget: Option<RunBudget>,
}
