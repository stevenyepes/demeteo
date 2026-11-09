//! What a dispatch to a runner signs beside the `start_feature`: the
//! repository and Workflow version a desktop reported, resolved from that
//! desktop's last snapshot (`docs/HUB.md` §6.4, decision 64 *Hub-started
//! runner runs*).
//!
//! A runner keeps no copy of a desktop's Projects or Workflows, so whatever
//! is signed here is all it will run. Every miss is therefore a refusal and
//! never a guess: a Project or Workflow the snapshot does not name, a
//! repository it cannot be cloned from as given, or a definition the
//! desktop's scrubber already rewrote.

use super::snapshot_view::version_name;
use crate::auth::handlers::ApiError;
use axum::http::StatusCode;
use demeteo_hub_protocol::{
    ProjectSnapshot, RunnerRepository, RunnerStart, RunnerWorkflow, Snapshot,
    WorkflowVersionSnapshot,
};

/// What core's `secret_scrub` writes in place of a secret it found. A
/// definition holding it is not the one the desktop runs, and a runner
/// handed it would run a `***` where a token or a command used to be. A
/// prompt that holds the three characters as its own text is refused too:
/// the Hub cannot tell the two apart.
pub const SCRUB_MASK: &str = "***";

/// Why a dispatch to a runner resolved nothing to sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchRefusal {
    /// The source's snapshot names no Project of the body's `project_id`, or
    /// there is no readable snapshot at all.
    UnknownProject,
    /// The snapshot names no version of the body's `workflow_id`.
    UnknownWorkflow,
    /// The Project and Workflow are there, but not as a runner can run them:
    /// no `provider_kind`, a `remote_url` that is not `https://host/owner/repo`,
    /// or a definition holding [`SCRUB_MASK`].
    ProjectNotDispatchable,
}

impl From<DispatchRefusal> for ApiError {
    fn from(refusal: DispatchRefusal) -> Self {
        let code = match refusal {
            DispatchRefusal::UnknownProject => "unknown_project",
            DispatchRefusal::UnknownWorkflow => "unknown_workflow",
            DispatchRefusal::ProjectNotDispatchable => "project_not_dispatchable",
        };
        ApiError::new(StatusCode::CONFLICT, code)
    }
}

/// The start a runner is signed for `project_id` and `workflow_id`, read from
/// `snapshot`, the last one `source_instance_id` sent. The Workflow is pinned
/// at its highest `version` in the snapshot, which is the one the browser was
/// offered (§6.4 *An instance's workflows are graph-only*).
pub fn runner_start(
    source_instance_id: &str,
    snapshot: Option<&Snapshot>,
    project_id: &str,
    workflow_id: &str,
) -> Result<RunnerStart, DispatchRefusal> {
    let snapshot = snapshot.ok_or(DispatchRefusal::UnknownProject)?;
    let project = snapshot
        .projects
        .iter()
        .find(|project| project.id == project_id)
        .ok_or(DispatchRefusal::UnknownProject)?;
    let version =
        latest(&snapshot.workflows, workflow_id).ok_or(DispatchRefusal::UnknownWorkflow)?;
    let repository = repository(project).ok_or(DispatchRefusal::ProjectNotDispatchable)?;
    if version.steps_json.contains(SCRUB_MASK) {
        return Err(DispatchRefusal::ProjectNotDispatchable);
    }
    Ok(RunnerStart {
        source_instance_id: source_instance_id.to_owned(),
        repository,
        workflow: RunnerWorkflow {
            workflow_id: version.workflow_id.clone(),
            version: version.version,
            name: version_name(version),
            steps_json: version.steps_json.clone(),
        },
    })
}

fn latest<'a>(
    versions: &'a [WorkflowVersionSnapshot],
    workflow_id: &str,
) -> Option<&'a WorkflowVersionSnapshot> {
    versions
        .iter()
        .filter(|version| version.workflow_id == workflow_id)
        .max_by_key(|version| version.version)
}

/// The desktop builds `remote_url` as `https://{host}/{repo_path}` from the
/// provider and the repository, never from git (core
/// `domain/hub_socket/snapshot.rs`), so this is the one shape to undo. The
/// provider is the snapshot's word alone: a host does not say which provider
/// serves it.
fn repository(project: &ProjectSnapshot) -> Option<RunnerRepository> {
    let provider_kind = project.provider_kind.as_deref().filter(|k| !k.is_empty())?;
    let rest = project.remote_url.as_deref()?.strip_prefix("https://")?;
    let (host, full_name) = rest.split_once('/')?;
    let well_formed = !host.is_empty()
        && full_name.contains('/')
        && full_name.split('/').all(|segment| !segment.is_empty());
    let doubtful = |c: char| c == '@' || c.is_whitespace() || c.is_control();
    if !well_formed || rest.contains(doubtful) {
        return None;
    }
    Some(RunnerRepository {
        provider_kind: provider_kind.to_owned(),
        host: host.to_owned(),
        full_name: full_name.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(remote_url: Option<&str>, provider_kind: Option<&str>) -> ProjectSnapshot {
        ProjectSnapshot {
            id: "p-1".to_owned(),
            name: "Project".to_owned(),
            remote_url: remote_url.map(str::to_owned),
            provider_kind: provider_kind.map(str::to_owned),
        }
    }

    fn version(workflow_id: &str, number: u32, steps_json: &str) -> WorkflowVersionSnapshot {
        WorkflowVersionSnapshot {
            id: format!("{workflow_id}-v{number}"),
            workflow_id: workflow_id.to_owned(),
            version: number,
            steps_json: steps_json.to_owned(),
            definition_json: Some(format!(
                r#"{{"name":"Standard v{number}","nodes":[],"edges":[]}}"#
            )),
            note: None,
            created_at: 1_700_000_000,
        }
    }

    fn snapshot(project: ProjectSnapshot, workflows: Vec<WorkflowVersionSnapshot>) -> Snapshot {
        Snapshot {
            v: 1,
            projects: vec![project],
            workflows,
            run_shape: Vec::new(),
            features: Vec::new(),
            harnesses: None,
            harness_catalog: None,
        }
    }

    fn dispatchable() -> Snapshot {
        snapshot(
            project(Some("https://github.com/acme/widgets"), Some("github")),
            vec![
                version("w-1", 3, r#"[{"v":3}]"#),
                version("w-1", 7, r#"[{"v":7}]"#),
                version("w-1", 5, r#"[{"v":5}]"#),
                version("w-2", 9, r#"[{"other":true}]"#),
            ],
        )
    }

    #[test]
    fn a_runner_start_pins_the_highest_version_and_splits_the_remote() {
        let start = runner_start("src-1", Some(&dispatchable()), "p-1", "w-1");

        assert_eq!(
            start,
            Ok(RunnerStart {
                source_instance_id: "src-1".to_owned(),
                repository: RunnerRepository {
                    provider_kind: "github".to_owned(),
                    host: "github.com".to_owned(),
                    full_name: "acme/widgets".to_owned(),
                },
                workflow: RunnerWorkflow {
                    workflow_id: "w-1".to_owned(),
                    version: 7,
                    name: "Standard v7".to_owned(),
                    steps_json: r#"[{"v":7}]"#.to_owned(),
                },
            })
        );
    }

    #[test]
    fn a_nested_group_and_a_host_with_a_port_are_kept_whole() {
        let snapshot = snapshot(
            project(
                Some("https://gitlab.example:8443/group/sub/repo"),
                Some("gitlab"),
            ),
            vec![version("w-1", 1, "[]")],
        );

        let repository =
            runner_start("src-1", Some(&snapshot), "p-1", "w-1").map(|start| start.repository);

        assert_eq!(
            repository,
            Ok(RunnerRepository {
                provider_kind: "gitlab".to_owned(),
                host: "gitlab.example:8443".to_owned(),
                full_name: "group/sub/repo".to_owned(),
            })
        );
    }

    #[test]
    fn a_project_or_workflow_the_snapshot_does_not_name_is_refused_by_which() {
        let snapshot = dispatchable();
        let cases = [
            (None, "p-1", "w-1", DispatchRefusal::UnknownProject),
            (
                Some(&snapshot),
                "p-2",
                "w-1",
                DispatchRefusal::UnknownProject,
            ),
            (
                Some(&snapshot),
                "p-1",
                "w-3",
                DispatchRefusal::UnknownWorkflow,
            ),
        ];
        for (snapshot, project_id, workflow_id, refusal) in cases {
            assert_eq!(
                runner_start("src-1", snapshot, project_id, workflow_id),
                Err(refusal),
                "{project_id} {workflow_id}"
            );
        }
    }

    #[test]
    fn a_project_a_runner_cannot_clone_as_given_is_not_dispatchable() {
        let url = "https://github.com/acme/widgets";
        let cases = [
            ("no provider", project(Some(url), None)),
            ("empty provider", project(Some(url), Some(""))),
            ("no remote", project(None, Some("github"))),
            (
                "not https",
                project(Some("http://github.com/acme/widgets"), Some("github")),
            ),
            (
                "scp form",
                project(Some("git@github.com:acme/widgets"), Some("github")),
            ),
            (
                "no owner",
                project(Some("https://github.com/widgets"), Some("github")),
            ),
            (
                "no path",
                project(Some("https://github.com"), Some("github")),
            ),
            (
                "no host",
                project(Some("https:///acme/widgets"), Some("github")),
            ),
            (
                "empty segment",
                project(Some("https://github.com/acme//widgets"), Some("github")),
            ),
            (
                "trailing slash",
                project(Some("https://github.com/acme/widgets/"), Some("github")),
            ),
            (
                "userinfo",
                project(Some("https://tok@github.com/acme/widgets"), Some("github")),
            ),
            (
                "whitespace",
                project(Some("https://github.com/acme/wid gets"), Some("github")),
            ),
        ];
        for (name, project) in cases {
            let snapshot = snapshot(project, vec![version("w-1", 1, "[]")]);
            assert_eq!(
                runner_start("src-1", Some(&snapshot), "p-1", "w-1"),
                Err(DispatchRefusal::ProjectNotDispatchable),
                "{name}"
            );
        }
    }

    #[test]
    fn a_latest_version_the_desktop_scrubbed_is_not_dispatchable() {
        let mut snapshot = dispatchable();
        snapshot
            .workflows
            .push(version("w-1", 8, r#"[{"prompt_template":"use ***"}]"#));

        assert_eq!(
            runner_start("src-1", Some(&snapshot), "p-1", "w-1"),
            Err(DispatchRefusal::ProjectNotDispatchable)
        );
    }

    #[test]
    fn an_unknown_workflow_is_named_before_the_project_is_judged_dispatchable() {
        let snapshot = snapshot(project(None, None), Vec::new());

        assert_eq!(
            runner_start("src-1", Some(&snapshot), "p-1", "w-1"),
            Err(DispatchRefusal::UnknownWorkflow)
        );
    }
}
