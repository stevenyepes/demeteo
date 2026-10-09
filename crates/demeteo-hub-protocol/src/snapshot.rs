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

/// Everything the Hub knows about an instance's Projects. A snapshot replaces
/// the previous one wholesale; the Hub never merges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub v: u32,
    pub projects: Vec<ProjectSnapshot>,
    pub workflows: Vec<WorkflowVersionSnapshot>,
    pub run_shape: Vec<RunShapeSettings>,
}
