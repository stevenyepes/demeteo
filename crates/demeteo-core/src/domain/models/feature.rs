use crate::domain::attachment::AttachedFile;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::harness_baseline::HarnessBaseline;
use crate::domain::ids::{
    FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId, WorkflowId, WorkflowVersionId,
};
use crate::domain::models::EffortLevel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Feature {
    pub id: FeatureId,
    pub project_id: ProjectId,
    pub workflow_id: Option<WorkflowId>,
    /// The workflow version this feature runs (decision 38, V33):
    /// resolved once at `feature_start` and read back by every resume,
    /// replay, and (Phase 2) the run-mode canvas, so editing the
    /// workflow mid-run never changes a running graph. `None` on
    /// pre-V33 rows; the run path backfills it by pinning latest on
    /// first resolve.
    #[serde(default)]
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub title: String,
    /// The rich prompt body the user typed at launch (rendered into the
    /// agent's `{{feature_description}}`). Persisted on the row (migration
    /// V27) so the pipeline view and home cards can show what the run does,
    /// not only its short `title`. Serde-defaults to `""` for specs/rows
    /// written before V27.
    #[serde(default)]
    pub description: String,
    pub status: String,
    pub total_cost: f64,
    pub duration: String,
    #[serde(default)]
    pub tokens: i64,
    pub created_at: i64,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Feature-wide effort override chosen at launch. `None` = inherit
    /// (workflow step value → project default → [`EffortLevel::DEFAULT`]).
    /// See migration V29.
    #[serde(default)]
    pub effort: Option<EffortLevel>,
    #[serde(default)]
    pub mr_url: Option<String>,
    #[serde(default)]
    pub mr_state: Option<String>,
    /// PR title/body authored by the `finalize` step's agent, read back by
    /// whoever opens the PR (the driver on the desktop, `demeteo-runner` when
    /// headless). `None` on features whose workflow has no finalize step —
    /// those fall back to the publisher's own default title/body.
    #[serde(default)]
    pub pr_title: Option<String>,
    #[serde(default)]
    pub pr_body: Option<String>,
    /// Per-feature override for the project's `commit_artifacts`
    /// setting. `None` = inherit from `ProjectSettings::commit_artifacts`.
    /// The StartFeatureModal exposes this as a toggle in the advanced
    /// section. See migration V12 and `commit_worktree_changes`.
    #[serde(default)]
    pub commit_artifacts: Option<bool>,
    /// Per-run override of the loop iteration budget. `None` = inherit the
    /// project default (`ProjectSettings::default_loop_iterations`) or the
    /// engine default (3). See migration V13.
    #[serde(default)]
    pub loop_iterations: Option<u32>,
    /// Per-run override of the per-turn dollar budget passed to the agent as
    /// `--max-budget-usd`. `None` = inherit the project default
    /// (`ProjectSettings::default_max_budget_usd`) or the engine default
    /// ([`crate::domain::agent_session::budget::DEFAULT_MAX_BUDGET_USD`]).
    /// This is the *base*
    /// budget for the primary coding turn; the bounded role turns (triage,
    /// finalize, verifier, planner) each get a fixed fraction of it. See
    /// migration V30.
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    /// Per-step agent/model overrides chosen at launch, snapshotted on the
    /// feature so workflow/project edits don't affect an in-flight run.
    /// Empty = every step inherits the workflow/project defaults.
    #[serde(default)]
    pub step_overrides: Vec<StepOverride>,

    /// Per-feature user attachments (images, files) — owned by the
    /// feature run. Stored as a JSON column on the feature row
    /// (`features.attachments_json`, migration V19) rather than a
    /// separate table so feature cleanup (auto-delete branch)
    /// releases the attachment lifetime implicitly. The on-disk
    /// file content lives in `FsAttachmentStore` at
    /// `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>`
    /// and is dropped by `FsAttachmentStore::clear_feature` when the
    /// feature is purged.
    #[serde(default)]
    pub attachments: Vec<AttachedFile>,

    /// What the project's harnesses said at this run's base commit
    /// (decision 44, `features.harness_baseline_json`, migration V37).
    /// Validate subtracts against it so a pre-existing red suite is not
    /// attributed to the feature.
    ///
    /// `None` = **no baseline was measured**, which is emphatically not
    /// "everything was green" — see [`HarnessBaseline`]. It rides on the
    /// `Feature` rather than in a side table so it replicates to the
    /// desktop on a detached run along the path `pr_title` and `effort`
    /// already travel: `hydrate_shadow_feature` pulls the runner's whole
    /// `Feature` over the `get_feature` RPC.
    #[serde(default)]
    pub harness_baseline: Option<HarnessBaseline>,

    /// Where this run's branch was cut from (`features.origin_json`, V41).
    /// [`FeatureOrigin::DefaultBranch`] on every row written before V41,
    /// which is what those runs did.
    #[serde(default)]
    pub origin: FeatureOrigin,

    /// What the review diff is computed against. `None` = the project's
    /// default branch. Distinct from [`Feature::origin`]: a run started from
    /// a PR head reviews against the branch it will merge into, not against
    /// the snapshot it began at. See migration V41.
    #[serde(default)]
    pub diff_base_branch: Option<String>,

    /// The branch this run actually works on, written down at cut time so the
    /// call sites that re-derive `{branch_prefix}{feature_id}` can read it
    /// instead — mid-run edits to `branch_prefix` otherwise split a live run's
    /// branch between the worktree and the publisher. `None` on pre-V41 rows,
    /// which keep deriving. See migration V41.
    #[serde(default)]
    pub resolved_branch: Option<String>,
}

impl Feature {
    /// The branch this run works on.
    ///
    /// The single reader every site that used to spell
    /// `format!("{branch_prefix}{feature_id}")` goes through, so a
    /// `branch_prefix` edited while the run is live can no longer move the
    /// publisher's branch off the worktree's. The derivation survives only as
    /// the answer for rows written before V41 recorded one.
    pub fn run_branch(&self, branch_prefix: &str) -> String {
        self.resolved_branch
            .clone()
            .unwrap_or_else(|| self.origin.branch_to_cut(branch_prefix, self.id.as_str()))
    }
}

/// A per-step agent/model/effort override selected when launching a feature.
/// Any field may be `None`, meaning "inherit" for that dimension.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StepOverride {
    pub step_id: String,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<EffortLevel>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepExecution {
    pub id: StepExecutionId,
    pub feature_id: FeatureId,
    pub step_id: StepId,
    pub step_index: u32,
    pub step_kind: String,
    pub status: String,
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub tokens: Option<i64>,
    pub wall_clock_secs: Option<u64>,
    #[serde(default)]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub artifact_paths: Vec<String>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub iteration_count: u32,
    /// Prompt-cache read tokens billed at the discounted rate for this
    /// step (last-turn snapshot, not aggregated into `tokens`).
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    /// Prompt-cache creation tokens (priced above base input) for this
    /// step (last-turn snapshot, not aggregated into `tokens`).
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    /// Normalized fingerprint of the last failing harness/prepare output for
    /// this step (C6). Set by the harness gate on a `Verdict` failure so the
    /// *next* attempt can detect a failure that reproduces unchanged and route
    /// it to regression-vs-environment triage. `None` until the first failure.
    #[serde(default)]
    pub last_failure_fingerprint: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubtaskRun {
    pub id: String,
    pub feature_id: FeatureId,
    pub step_execution_id: StepExecutionId,
    pub subtask_id: String,
    pub agent_id: Option<String>,
    pub worktree_path: String,
    pub branch: String,
    pub status: String,
    pub cost_usd: f64,
    #[serde(default)]
    pub tokens: i64,
    pub error_message: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GateDecision {
    pub id: GateDecisionId,
    pub step_execution_id: StepExecutionId,
    /// None = pending. "approve" | "redirect" | "cancel"
    pub decision: Option<String>,
    /// Feedback / redirect instructions provided by the user.
    pub feedback: Option<String>,
    pub created_at: i64,
}
