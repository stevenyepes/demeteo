use std::borrow::Cow;

use demeteo_hub_protocol::{
    Effort, ProjectSnapshot, RunShapeSettings, Snapshot, WorkflowVersionSnapshot, PROTOCOL_VERSION,
};
use serde_json::Value;

use crate::domain::git_push::without_userinfo;
use crate::domain::models::{
    EffortLevel, Project, ProjectSettings, ProviderInstance, Repository, WorkflowVersion,
};
use crate::shared::secret_scrub::scrub_secrets;

/// The Hub's `MAX_MESSAGE_BYTES`, mirrored for the same reason as
/// [`MAX_FEATURE_ID_BYTES`](super::MAX_FEATURE_ID_BYTES): an encoded snapshot
/// over it is refused by the Hub, so the session does not send one.
pub const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

/// The default debounce window between a waking write and the rebuilt
/// snapshot, in seconds. Injectable per session.
pub const SNAPSHOT_DEBOUNCE_SECS: u64 = 2;

/// What the builder reads. Domain types only — gathering them is the
/// adapter's job.
pub struct SnapshotInput {
    pub projects: Vec<ProjectInput>,
    /// The latest stored version of each Workflow, not every version: every
    /// save appends one, and sending them all grows towards
    /// [`MAX_SNAPSHOT_BYTES`].
    pub workflows: Vec<WorkflowVersion>,
    pub providers: Vec<ProviderInstance>,
}

pub struct ProjectInput {
    pub project: Project,
    pub settings: Option<ProjectSettings>,
    pub repositories: Vec<Repository>,
}

/// The `Snapshot` an instance reports (docs/HUB.md §5).
///
/// Every outbound field is copied by name from an allowlist; no domain struct
/// is forwarded through serde, so a field added to `Project` or
/// `ProjectSettings` later cannot cross until someone adds it here. Anything
/// holding a `'\0'` is dropped, since the Hub refuses the whole frame for one.
/// Output is sorted by project id, and workflows by workflow id, so equal
/// inputs give equal snapshots, which is what makes the session's "same as last sent?" comparison meaningful.
pub fn snapshot(input: &SnapshotInput) -> Snapshot {
    let mut projects: Vec<ProjectSnapshot> = input
        .projects
        .iter()
        .filter_map(|p| project(p, &input.providers))
        .collect();
    projects.sort_by(|a, b| a.id.cmp(&b.id));
    let mut run_shape: Vec<RunShapeSettings> = input
        .projects
        .iter()
        .filter_map(|p| run_shape(p.settings.as_ref()?))
        .collect();
    run_shape.sort_by(|a, b| a.project_id.cmp(&b.project_id));
    let mut workflows: Vec<WorkflowVersionSnapshot> =
        input.workflows.iter().filter_map(workflow).collect();
    workflows.sort_by(|a, b| a.workflow_id.cmp(&b.workflow_id));
    Snapshot {
        v: PROTOCOL_VERSION,
        projects,
        workflows,
        run_shape,
    }
}

fn project(input: &ProjectInput, providers: &[ProviderInstance]) -> Option<ProjectSnapshot> {
    let out = ProjectSnapshot {
        id: input.project.id.as_str().to_owned(),
        name: input.project.name.clone(),
        remote_url: remote_url(input.repositories.first(), providers),
    };
    let strings = [Some(&out.id), Some(&out.name), out.remote_url.as_ref()];
    no_nul(strings).then_some(out)
}

/// Derived from the provider's host and the repo path, never asked of git.
/// Only the first repository is reported, as `resolve_target_repo` picks it,
/// because `ProjectSnapshot` has one `remote_url`. `ProviderInstance.username`
/// is never read: it is an account name, not part of the remote.
///
/// Fails closed. A host that smuggles userinfo `HttpRemote::parse` cannot
/// see whole — a password with an unencoded `/` ends the authority early —
/// leaves an `@` behind, and that is `None` rather than a guess.
fn remote_url(repo: Option<&Repository>, providers: &[ProviderInstance]) -> Option<String> {
    let repo = repo?;
    if repo.repo_path.is_empty() {
        return None;
    }
    let provider = providers.iter().find(|p| p.id == repo.provider_id)?;
    let url = without_userinfo(&format!("https://{}/{}", provider.host, repo.repo_path))?;
    let doubtful = |c: char| c == '@' || c.is_whitespace() || c.is_control();
    (!url.contains(doubtful)).then_some(url)
}

/// `None` when either document does not parse: a version the Hub cannot
/// read whole is not reported in part.
fn workflow(version: &WorkflowVersion) -> Option<WorkflowVersionSnapshot> {
    let definition_json = match &version.definition_json {
        Some(doc) => Some(outbound_document(doc)?),
        None => None,
    };
    let out = WorkflowVersionSnapshot {
        id: version.id.as_str().to_owned(),
        workflow_id: version.workflow_id.as_str().to_owned(),
        version: version.version,
        steps_json: outbound_document(&version.steps_json)?,
        definition_json,
        note: version.note.clone(),
        created_at: version.created_at.div_euclid(1000),
    };
    let strings = [Some(&out.id), Some(&out.workflow_id), out.note.as_ref()];
    no_nul(strings).then_some(out)
}

/// `doc` with every `cwd` key removed at any depth — a step's working
/// directory is a local path — and every remaining string scrubbed.
/// Command and prompt templates do cross, scrubbed but with any path in
/// their free text intact: the carried default of docs/HUB.md §11 item 11.
///
/// A `\u0000` escape is a NUL once the Hub decodes the document, so one
/// anywhere drops the version as a raw NUL would.
fn outbound_document(doc: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(doc).ok()?;
    if !strip(&mut value) {
        return None;
    }
    serde_json::to_string(&value).ok()
}

/// Removes and scrubs in place; `false` on the first NUL, after which
/// `value` is half-walked and only fit to drop.
fn strip(value: &mut Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.remove("cwd");
            fields.keys().all(|k| !k.contains('\0')) && fields.values_mut().all(strip)
        }
        Value::Array(items) => items.iter_mut().all(strip),
        Value::String(text) => {
            if let Cow::Owned(scrubbed) = scrub_secrets(text) {
                *text = scrubbed;
            }
            !text.contains('\0')
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
    }
}

/// The run-shape subset of `settings`. The fields read here are the whole
/// read: `worktree_strategy`, `artifact_subdir` and the rest hold commands
/// and local paths, which never leave an instance (AGENTS §2).
fn run_shape(settings: &ProjectSettings) -> Option<RunShapeSettings> {
    let out = RunShapeSettings {
        project_id: settings.project_id.as_str().to_owned(),
        default_agent_kind: settings.default_agent_kind.clone(),
        default_model: settings.default_model.clone(),
        default_effort: settings.default_effort.map(effort),
        default_workflow_id: settings.default_workflow_id.clone(),
        default_loop_iterations: settings.default_loop_iterations,
        default_max_budget_cents: settings.default_max_budget_usd.and_then(cents),
    };
    let strings = [
        Some(&out.project_id),
        out.default_agent_kind.as_ref(),
        out.default_model.as_ref(),
        out.default_workflow_id.as_ref(),
    ];
    no_nul(strings).then_some(out)
}

/// Exhaustive rather than a serde round-trip: the protocol crate mirrors
/// core's spellings without sharing them, and a level added to
/// `EffortLevel` must fail to compile here instead of failing to parse at
/// runtime and silently dropping.
fn effort(level: EffortLevel) -> Effort {
    match level {
        EffortLevel::Low => Effort::Low,
        EffortLevel::Medium => Effort::Medium,
        EffortLevel::High => Effort::High,
        EffortLevel::XHigh => Effort::Xhigh,
        EffortLevel::Max => Effort::Max,
    }
}

/// `None` for a stored budget no launch could honour; the `as` saturates.
fn cents(usd: f64) -> Option<u64> {
    (usd.is_finite() && usd >= 0.0).then(|| (usd * 100.0).round() as u64)
}

fn no_nul<'a>(strings: impl IntoIterator<Item = Option<&'a String>>) -> bool {
    strings.into_iter().flatten().all(|s| !s.contains('\0'))
}
