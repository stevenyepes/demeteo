//! The two lookups the `{{harness_baseline}}` block needs, against one strict
//! double.
//!
//! `harness_briefing` reads exactly one port, so this file stubs exactly one —
//! no `ExecutionDriver`, and the double **errors on every method it was not
//! explicitly told to answer** (AGENTS.md §7: a double that answers everything
//! successfully asserts against a default rather than an answer).

use super::*;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, StepId, WorkflowId};
use crate::domain::models::{Feature, ProjectSettings, StepConfig, WorktreeStrategy};
use crate::domain::verifier::VerifierConfig;
use crate::ports::db::ProjectRepository;

const P_ID: &str = "p-brief";

/// Answers `get_settings` for one project and **errors on everything else**,
/// including a lookup of any other project.
struct SettingsDouble {
    settings: Option<ProjectSettings>,
}

impl SettingsDouble {
    fn readable(strategy: WorktreeStrategy) -> Self {
        Self {
            settings: Some(ProjectSettings {
                project_id: ProjectId::from(P_ID.to_string()),
                worktree_strategy: strategy,
                conflict_policy: "manual".to_string(),
                feature_lifecycle: "keep".to_string(),
                default_agent_kind: None,
                default_model: None,
                default_effort: None,
                default_workflow_id: None,
                default_loop_iterations: None,
                default_max_budget_usd: None,
                artifact_subdir: "artifacts/".to_string(),
                commit_artifacts: false,
                review_entrypoint: None,
                sync_resolver_agent_kind: None,
                sync_resolver_model: None,
                sync_resolver_effort: None,
                sync_review_before_push: None,
            }),
        }
    }

    fn unreadable() -> Self {
        Self { settings: None }
    }
}

macro_rules! unscripted {
    ($($name:ident($($arg:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $(_: $arg),*) -> $ret {
            Err(concat!("unscripted ProjectRepository::", stringify!($name)).to_string())
        })*
    };
}

impl ProjectRepository for SettingsDouble {
    fn get_settings(&self, project_id: &ProjectId) -> Result<Option<ProjectSettings>, String> {
        if project_id.0 != P_ID {
            return Err(format!("unscripted project {}", project_id.0));
        }
        match &self.settings {
            Some(s) => Ok(Some(s.clone())),
            None => Err("settings row could not be read".to_string()),
        }
    }

    unscripted! {
        get_projects() -> Result<Vec<crate::domain::models::Project>, String>;
        get_project(&ProjectId) -> Result<Option<crate::domain::models::Project>, String>;
        add(crate::domain::models::Project) -> Result<(), String>;
        update(crate::domain::models::Project) -> Result<(), String>;
        update_status(&ProjectId, &str) -> Result<(), String>;
        delete(&ProjectId) -> Result<(), String>;
        delete_repositories_for(&ProjectId) -> Result<(), String>;
        add_repository(crate::domain::models::Repository) -> Result<(), String>;
        get_repositories_for(&ProjectId) -> Result<Vec<crate::domain::models::Repository>, String>;
        save_settings(ProjectSettings) -> Result<(), String>;
        list_workflow_overrides(&ProjectId)
            -> Result<Vec<crate::domain::models::ProjectWorkflowOverride>, String>;
        list_overrides_for_workflow(&ProjectId, &WorkflowId)
            -> Result<Vec<crate::domain::models::ProjectWorkflowOverride>, String>;
        upsert_workflow_override(crate::domain::models::ProjectWorkflowOverride)
            -> Result<(), String>;
    }
}

fn strategy(test_command: Option<&str>) -> WorktreeStrategy {
    WorktreeStrategy {
        default_branch: "main".to_string(),
        branch_prefix: "demeteo/features/".to_string(),
        test_command: test_command.map(str::to_string),
        build_command: None,
        coverage_command: None,
        conventions_file: None,
        pr_template: None,
        harnesses: None,
        validation_gates: None,
        prepare_command: None,
        extra_writable_paths: Vec::new(),
    }
}

fn feature() -> Feature {
    Feature {
        id: FeatureId::from("f-brief"),
        project_id: ProjectId::from(P_ID),
        workflow_id: Some(WorkflowId::from("w-1")),
        workflow_version_id: None,
        title: "briefing feature".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: 1_700_000_000,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: None,
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

/// A step carrying a verifier that pins `names` as its gates.
fn verifying_step(id: &str, names: &[&str]) -> StepConfig {
    StepConfig {
        id: StepId::from(id.to_string()),
        kind: "agent".to_string(),
        title: id.to_string(),
        verifier: Some(VerifierConfig {
            agent_kind: None,
            model: None,
            effort: None,
            instructions: String::new(),
            harness_names: names.iter().map(|n| n.to_string()).collect(),
            verdict_key: "verdict".to_string(),
        }),
        ..StepConfig::default()
    }
}

fn plain_step(id: &str) -> StepConfig {
    StepConfig {
        id: StepId::from(id.to_string()),
        kind: "agent".to_string(),
        title: id.to_string(),
        ..StepConfig::default()
    }
}

#[test]
fn no_feature_yields_no_block_and_asks_nothing() {
    let projects = SettingsDouble::unreadable();
    assert_eq!(
        harness_briefing(&projects, &[plain_step("s-spec")], 600, None),
        "",
        "with no feature there is no project to look settings up for"
    );
}

/// A prompt section describing a harness this project does not have is worse
/// than no section, so an unreadable settings row is an empty block — never a
/// guess at what the gates might be.
#[test]
fn unreadable_settings_yield_no_block_rather_than_a_guess() {
    let projects = SettingsDouble::unreadable();
    let rendered = harness_briefing(
        &projects,
        &[verifying_step("s-validate", &["unit"])],
        600,
        Some(&feature()),
    );
    assert_eq!(rendered, "");
}

#[test]
fn gates_are_collected_from_every_step_that_carries_a_verifier() {
    let mut harnesses = std::collections::HashMap::new();
    harnesses.insert("lint".to_string(), "npm run lint".to_string());
    harnesses.insert("unit".to_string(), "npm run unit".to_string());
    let projects = SettingsDouble::readable(WorktreeStrategy {
        harnesses: Some(harnesses),
        ..strategy(Some("npm test"))
    });

    let rendered = harness_briefing(
        &projects,
        &[
            plain_step("s-spec"),
            verifying_step("s-validate", &["lint"]),
            verifying_step("s-critic", &["unit"]),
        ],
        600,
        Some(&feature()),
    );

    assert!(
        rendered.contains("npm run lint") && rendered.contains("npm run unit"),
        "telling `s-spec` about a subset of the gates that will run is the same \
         class of lie as telling it about none of them; got:\n{rendered}"
    );
}

#[test]
fn a_gate_declared_by_two_steps_is_named_once() {
    let mut harnesses = std::collections::HashMap::new();
    harnesses.insert("unit".to_string(), "npm run unit".to_string());
    let projects = SettingsDouble::readable(WorktreeStrategy {
        harnesses: Some(harnesses),
        ..strategy(None)
    });

    let rendered = harness_briefing(
        &projects,
        &[
            verifying_step("s-validate", &["unit"]),
            verifying_step("s-critic", &["unit"]),
        ],
        600,
        Some(&feature()),
    );

    assert_eq!(
        rendered.matches("`npm run unit`").count(),
        1,
        "the union is deduplicated by name; got:\n{rendered}"
    );
}
