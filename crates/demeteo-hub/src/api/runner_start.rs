//! What a dispatch to a runner signs beside the `start_feature`: the
//! repository of a Hub Project and an embedded starter Workflow
//! (`docs/HUB.md` §6.4). Both are the Hub's own, never a desktop's, which
//! supersedes decision 65's resolution through a source desktop's snapshot.
//!
//! A runner keeps no Projects or Workflows and runs with no project settings,
//! so whatever is signed here is all it will run, and the Hub Project's
//! run-shape defaults are filled in before signing or not at all. Every miss
//! is a refusal and never a guess.

use crate::auth::handlers::ApiError;
use crate::projects::verdict::runner_ready;
use crate::store::hub_projects::HubProjectRow;
use crate::workflows::Starter;
use axum::http::StatusCode;
use demeteo_hub_protocol::{RunnerRepository, RunnerStart, RunnerWorkflow, StartFeature};

/// What every Hub-built [`RunnerStart`] carries as `source_instance_id`. No
/// runner reads the field, and every deployed runner requires it.
pub const HUB_ORIGIN: &str = "hub";

/// Why a dispatch to a runner resolved nothing to sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchRefusal {
    /// The body's `project_id` names no Hub Project of the tenant.
    UnknownProject,
    /// The body's `workflow_id` names no embedded starter.
    UnknownWorkflow,
    /// The Hub Project is one a runner cannot clone yet: GitLab, until a
    /// runner can be minted a GitLab token (decision 71).
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

/// The start a runner is signed for `project` and `starter`, each `None`
/// when the body named none the Hub holds. The Workflow is judged first.
pub fn runner_start(
    project: Option<&HubProjectRow>,
    starter: Option<&Starter>,
) -> Result<RunnerStart, DispatchRefusal> {
    let starter = starter.ok_or(DispatchRefusal::UnknownWorkflow)?;
    let fields = &project.ok_or(DispatchRefusal::UnknownProject)?.fields;
    if !runner_ready(&fields.provider_kind) {
        return Err(DispatchRefusal::ProjectNotDispatchable);
    }
    Ok(RunnerStart {
        source_instance_id: HUB_ORIGIN.to_owned(),
        repository: RunnerRepository {
            provider_kind: fields.provider_kind.clone(),
            host: fields.host.clone(),
            full_name: fields.full_name.clone(),
        },
        workflow: RunnerWorkflow {
            workflow_id: starter.id.clone(),
            version: starter.version,
            name: starter.name.clone(),
            steps_json: starter.steps_json.clone(),
        },
    })
}

/// `start` with each run-shape field the body left out taken from `row`'s
/// defaults. A field the body set is kept, whatever the project says.
pub fn run_shape_defaults(start: StartFeature, row: &HubProjectRow) -> StartFeature {
    let defaults = &row.fields;
    StartFeature {
        agent_kind: start
            .agent_kind
            .or_else(|| defaults.default_agent_kind.clone()),
        model: start.model.or_else(|| defaults.default_model.clone()),
        effort: start.effort.or(defaults.default_effort),
        max_budget_cents: start.max_budget_cents.or(defaults.default_max_budget_cents),
        ..start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::snapshot_view::WorkflowGraphView;
    use crate::store::hub_projects::{HubProjectFields, HubProjectId};
    use demeteo_hub_protocol::Effort;
    use time::OffsetDateTime;

    fn row(provider_kind: &str) -> HubProjectRow {
        HubProjectRow {
            id: HubProjectId::generate(),
            fields: HubProjectFields {
                name: "Widgets".to_owned(),
                provider_kind: provider_kind.to_owned(),
                host: "github.com".to_owned(),
                full_name: "acme/widgets".to_owned(),
                default_agent_kind: Some("codex".to_owned()),
                default_model: Some("gpt-5".to_owned()),
                default_effort: Some(Effort::High),
                default_workflow_id: None,
                default_max_budget_cents: Some(900),
            },
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn starter() -> Starter {
        Starter {
            id: "wf-starter-standard".to_owned(),
            name: "Standard".to_owned(),
            description: "Plan, build, review.".to_owned(),
            version: 42,
            steps_json: r#"[{"id":"s-plan"}]"#.to_owned(),
            graph: WorkflowGraphView {
                nodes: Vec::new(),
                edges: Vec::new(),
            },
        }
    }

    fn start() -> StartFeature {
        StartFeature {
            project_id: "p".to_owned(),
            workflow_id: "wf-starter-standard".to_owned(),
            title: "t".to_owned(),
            description: String::new(),
            agent_kind: None,
            model: None,
            effort: None,
            step_overrides: Vec::new(),
            max_budget_cents: None,
            max_wall_clock_secs: None,
            attachments: Vec::new(),
            runner: None,
        }
    }

    #[test]
    fn a_github_project_and_a_starter_sign_their_repository_and_steps_from_the_hub() {
        let start = runner_start(Some(&row("github")), Some(&starter()));

        assert_eq!(
            start,
            Ok(RunnerStart {
                source_instance_id: "hub".to_owned(),
                repository: RunnerRepository {
                    provider_kind: "github".to_owned(),
                    host: "github.com".to_owned(),
                    full_name: "acme/widgets".to_owned(),
                },
                workflow: RunnerWorkflow {
                    workflow_id: "wf-starter-standard".to_owned(),
                    version: 42,
                    name: "Standard".to_owned(),
                    steps_json: r#"[{"id":"s-plan"}]"#.to_owned(),
                },
            })
        );
    }

    #[test]
    fn the_signed_start_keeps_the_wire_shape_every_deployed_runner_reads() {
        let start = runner_start(Some(&row("github")), Some(&starter())).unwrap();
        let wire = serde_json::to_value(&start).unwrap();

        let keys = |value: &serde_json::Value| {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        assert_eq!(
            keys(&wire),
            ["repository", "source_instance_id", "workflow"]
        );
        assert_eq!(
            keys(&wire["repository"]),
            ["full_name", "host", "provider_kind"]
        );
        assert_eq!(
            keys(&wire["workflow"]),
            ["name", "steps_json", "version", "workflow_id"]
        );
    }

    #[test]
    fn each_miss_is_refused_by_which_and_the_workflow_is_judged_first() {
        let github = row("github");
        let gitlab = row("gitlab");
        let starter = starter();
        let cases = [
            (None, None, DispatchRefusal::UnknownWorkflow),
            (Some(&gitlab), None, DispatchRefusal::UnknownWorkflow),
            (None, Some(&starter), DispatchRefusal::UnknownProject),
            (
                Some(&gitlab),
                Some(&starter),
                DispatchRefusal::ProjectNotDispatchable,
            ),
        ];
        for (project, starter, refusal) in cases {
            assert_eq!(runner_start(project, starter), Err(refusal), "{refusal:?}");
        }
        assert!(runner_start(Some(&github), Some(&starter)).is_ok());
    }

    #[test]
    fn defaults_fill_only_what_the_body_left_out() {
        let filled = run_shape_defaults(start(), &row("github"));
        assert_eq!(
            (
                filled.agent_kind.as_deref(),
                filled.model.as_deref(),
                filled.effort,
                filled.max_budget_cents,
            ),
            (Some("codex"), Some("gpt-5"), Some(Effort::High), Some(900))
        );

        let asked = StartFeature {
            agent_kind: Some("claude-code".to_owned()),
            model: Some("opus".to_owned()),
            effort: Some(Effort::Low),
            max_budget_cents: Some(5),
            max_wall_clock_secs: Some(60),
            ..start()
        };
        assert_eq!(run_shape_defaults(asked.clone(), &row("github")), asked);

        let mut bare = row("github");
        bare.fields.default_agent_kind = None;
        bare.fields.default_model = None;
        bare.fields.default_effort = None;
        bare.fields.default_max_budget_cents = None;
        assert_eq!(run_shape_defaults(start(), &bare), start());
    }
}
