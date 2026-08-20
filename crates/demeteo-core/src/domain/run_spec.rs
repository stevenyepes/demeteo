//! `RunSpec` — the payload that drives a headless run
//! (docs/REMOTE_EXECUTION.md M1.2). Carries everything the runner
//! needs to bootstrap a project, ingest a workflow, and start a feature,
//! without depending on any state already present in the runner's own
//! (fresh) database — the laptop composes this from its own DB state and
//! ships it over. Reused as-is by the control-RPC `submit_run` payload
//! (M3) once the laptop⇄runner channel exists.

use serde::{Deserialize, Serialize};

use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::models::{EffortLevel, ProjectSettings, StepOverride};

/// A pre-launch attachment for a detached run. The laptop spools the
/// bytes onto the runner host over SFTP (`ExecutionPort::write_file_bytes`)
/// *before* `submit_run`, then references the spooled path here — raw
/// bytes never ride the line-JSON control RPC. The runner feeds these
/// into `feature_start` as path-based staged attachments and deletes the
/// spool directory when the run reaches a terminal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpecAttachment {
    /// Absolute path on the runner host where the bytes were spooled.
    pub staged_path: String,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub source_filename: Option<String>,
}

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

/// Per-run hard caps on token cost and wall-clock time (docs/REMOTE_EXECUTION.md
/// M5.2, docs/REMOTE_EXECUTION.md §5). Exceeding either cap cancels the run
/// as `over-budget` — unattended never auto-approves more spend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunBudget {
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub max_wall_clock_secs: Option<u64>,
}

/// A run's declared starting point, as it arrives over the wire.
///
/// [`FeatureOrigin`] is internally tagged, so a `kind` this build has never
/// heard of would otherwise fail the whole [`RunSpec`] parse and answer a
/// perfectly ordinary submit with a serde complaint about a field no operator
/// typed. Keeping the undecoded document instead lets
/// [`RunSpec::origin_to_honour`] refuse it by name.
///
/// The refusal is the point of the type. The two shapes that would make the
/// field parse everywhere — a `#[serde(other)]` arm on `FeatureOrigin`, or
/// decoding into `Option` and dropping what fails — both resolve an origin
/// this build cannot honour to [`FeatureOrigin::DefaultBranch`] and then *run
/// it*: the run succeeds, pushes, opens its PR, and the diff a reviewer reads
/// is against a tree nobody chose. There is no error anywhere in that
/// sequence, which is why the decoding has to keep the evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunOrigin {
    Supported(FeatureOrigin),
    /// The document as it arrived, so the refusal can quote it.
    Unsupported(serde_json::Value),
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
    /// Feature-wide effort for the run. Serde-defaults to `None` so specs
    /// from an older laptop still deserialize (and an older runner ignores
    /// the field, falling back to the agent's own default).
    #[serde(default)]
    pub effort: Option<EffortLevel>,
    /// Per-run override of the `on_failure` retry-loop budget. `None`
    /// inherits the engine default.
    pub loop_iterations: Option<u32>,
    /// Per-run override of the per-turn dollar budget (`--max-budget-usd`).
    /// `None` inherits the project default, then the engine default. Serde-
    /// defaults so specs from an older laptop still deserialize.
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    /// Per-step agent/model overrides chosen at launch — same rows
    /// `start_feature` accepts (migration V13). Serde-default so specs
    /// from older laptops (and to older runners) stay compatible.
    #[serde(default)]
    pub step_overrides: Vec<StepOverride>,
    /// Per-run override of the project's `commit_artifacts` setting
    /// (migration V12). `None` inherits the runner-side project default.
    #[serde(default)]
    pub commit_artifacts: Option<bool>,
    /// Attachments spooled onto the runner host before submit. See
    /// [`RunSpecAttachment`].
    #[serde(default)]
    pub attachments: Vec<RunSpecAttachment>,
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
    /// The launching client's project settings, so a detached run honors
    /// *its* harnesses/prepare-command/test-command/extra-writable-paths/
    /// lifecycle rather than runner-side re-detected defaults
    /// (docs/MULTI_CLIENT_RUNNER.md MC-D4 / P0.5, gap **f**). The runner
    /// overlays these onto the row it persists via `save_settings` *before*
    /// the shared `feature_start` reads it, resolving `default_branch` from
    /// this run's own base rather than from this copy. `None` (an old
    /// client) reproduces today's behavior exactly: detected strategy +
    /// engine defaults. `project_id` on the payload is ignored — the runner
    /// re-homes it onto the run's own project.
    #[serde(default)]
    pub project_settings: Option<ProjectSettings>,
    /// Where this run's branch is cut from (V41). `None` — an old client, or
    /// a launch that chose nothing — is [`FeatureOrigin::DefaultBranch`],
    /// which is what every detached run did before this field existed. Read
    /// through [`RunSpec::origin_to_honour`], never directly.
    #[serde(default)]
    pub origin: Option<RunOrigin>,
    /// What the run's review diff is measured against and what its PR
    /// targets, when that is not where it started — a run cut from a PR head
    /// reviews against the branch it will merge into. `None` = the origin's
    /// own base, then the project's default branch. See
    /// [`FeatureOrigin::base_branch`].
    #[serde(default)]
    pub diff_base_branch: Option<String>,
}

impl RunSpec {
    /// Where this run must start, or the reason to refuse it outright.
    ///
    /// "Absent is not green" applied to a run's own base: a spec naming an
    /// origin this build cannot resolve is refused, never folded to
    /// [`FeatureOrigin::DefaultBranch`]. A runner one release behind the
    /// client that submitted to it is the ordinary way to arrive here, and it
    /// is exactly the case where falling back looks harmless — the run would
    /// complete, and nothing but the diff would show that it reviewed the
    /// wrong tree.
    pub fn origin_to_honour(&self) -> Result<FeatureOrigin, String> {
        match &self.origin {
            None => Ok(FeatureOrigin::DefaultBranch),
            Some(RunOrigin::Supported(origin)) => Ok(origin.clone()),
            Some(RunOrigin::Unsupported(raw)) => Err(format!(
                "this runner cannot start a run from the origin the client sent ({raw}) — \
                 upgrade demeteo-runner on this machine"
            )),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/domain/run_spec.rs"]
mod tests;
