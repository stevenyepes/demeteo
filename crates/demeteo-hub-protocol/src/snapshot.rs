use serde::{Deserialize, Serialize};

use crate::Effort;

/// A Project as the Hub sees it: an id, a name, and where it came from.
///
/// There is no path field, deliberately. A local path names a user, a machine
/// and a directory layout, and local paths never leave an instance (AGENTS §2,
/// HUB.md §3). Do not add one — `deny_unknown_fields` and the key walk in
/// `tests/snapshot.rs` exist to make that edit fail loudly.
///
/// `remote_url` is carried as given: this crate cannot tell a URL with
/// userinfo from one without, so the desktop snapshot builder must strip
/// credentials before it constructs this type (HUB.md §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSnapshot {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub remote_url: Option<String>,
    /// `github` or `gitlab`, from the provider the Project's Repository names;
    /// what a Hub reads to start a runner on this Project, since a host does
    /// not say which provider serves it. `None` from a desktop that predates
    /// the field, and from a Project with no provider.
    #[serde(default)]
    pub provider_kind: Option<String>,
}

/// One stored version of a Workflow. Field names mirror `demeteo-core`'s
/// `WorkflowVersion`; the crate does not depend on core.
///
/// The two `*_json` documents are opaque here, and in core they hold each
/// step's `command`, `cwd`, `env_allowlist`, `prompt_template` and
/// `rework_prompt_template`. The desktop snapshot builder must not send the
/// stored documents unchanged: it strips `cwd` and any other local path before
/// constructing this type (HUB.md §3, §5). That does not make the rest safe —
/// `command` and the prompt templates are user-authored text that may embed a
/// path or a credential, which no field-level strip can detect. Whether they
/// may cross at all is open in HUB.md §11.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowVersionSnapshot {
    pub id: String,
    pub workflow_id: String,
    pub version: u32,
    pub steps_json: String,
    #[serde(default)]
    pub definition_json: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
}

/// The run-shape subset of a Project's settings — what a remote launch
/// pre-fills, and nothing else. Test commands, paths and credentials are not
/// run shape and have no field here.
///
/// `default_agent_kind` is a plain `String`, not an enum: harnesses are added
/// on the instance, and a closed list here would make a new harness a
/// protocol break for a field the Hub only echoes back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunShapeSettings {
    pub project_id: String,
    #[serde(default)]
    pub default_agent_kind: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_effort: Option<Effort>,
    #[serde(default)]
    pub default_workflow_id: Option<String>,
    #[serde(default)]
    pub default_loop_iterations: Option<u32>,
    #[serde(default)]
    pub default_max_budget_cents: Option<u64>,
}

/// A Feature as the Hub may name it in a view: what the user called it,
/// which Project it belongs to and which Workflow version it runs
/// (decision 70, HUB.md §5).
///
/// `title` is user-authored text, the one such field a snapshot carries. The
/// desktop snapshot builder caps and scrubs it; the description the user
/// typed at launch is not a title and has no field here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSnapshot {
    pub id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub workflow_id: Option<String>,
    /// The pinned version's number, which names one of
    /// [`Snapshot::workflows`] when the builder sent it.
    #[serde(default)]
    pub workflow_version: Option<u32>,
}

/// What an instance reports of itself: its Projects, their Workflows and run
/// shape, the Features it may name, and which harnesses it can run with which
/// models. A snapshot replaces the previous one wholesale; the Hub never
/// merges.
///
/// `features`, `harnesses`, `harness_catalog` and
/// [`ProjectSnapshot::provider_kind`] were added after the first Hub shipped
/// and default when absent, so a newer Hub reads an older instance's
/// snapshot. The reverse does not hold: `deny_unknown_fields` makes an older
/// Hub refuse a snapshot carrying any of them, so the Hub ships first
/// (HUB.md §11 item 8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub v: u32,
    pub projects: Vec<ProjectSnapshot>,
    pub workflows: Vec<WorkflowVersionSnapshot>,
    pub run_shape: Vec<RunShapeSettings>,
    #[serde(default)]
    pub features: Vec<FeatureSnapshot>,
    /// The harness kinds the instance has registered (`claude-code`,
    /// `opencode`, …), whether or not they are installed. What is known
    /// about each one is [`Snapshot::harness_catalog`]. `None` from an
    /// instance that predates the field, which is not the same as one with
    /// none registered.
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,
    /// The harnesses the instance found installed on its own machine, and
    /// the models each one lists there. `None` from an instance that
    /// predates the field or has not finished a probe yet; `Some([])` from
    /// one that probed and found nothing installed.
    #[serde(default)]
    pub harness_catalog: Option<Vec<HarnessCatalogEntry>>,
}

/// One installed harness and what it lists, as the instance last probed it.
///
/// `kind` is a plain `String` for the reason
/// [`RunShapeSettings::default_agent_kind`] is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessCatalogEntry {
    pub kind: String,
    pub models: Vec<ModelChoice>,
    #[serde(default)]
    pub effort_levels: Option<Vec<Effort>>,
    /// Where the list came from (`config`, `probed`, `bundled`). A bundled
    /// list is the instance's built-in guess, not what the harness answered.
    /// `None` with no models is a harness that has not yet answered a listing.
    #[serde(default)]
    pub source: Option<String>,
    /// Unix seconds.
    pub probed_at: i64,
}

/// A model a harness lists: the value a run passes and the name a picker
/// shows.
///
/// There is no `description`, deliberately: the instance's own description
/// of a model can name the config file it read, a path under the user's
/// home that never leaves an instance (AGENTS §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChoice {
    pub value: String,
    pub name: String,
}
