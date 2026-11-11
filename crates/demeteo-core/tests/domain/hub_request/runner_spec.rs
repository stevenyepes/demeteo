// The run a runner builds from a Hub start. `super` =
// `domain::hub_request::runner_spec`.

use super::*;
use crate::domain::models::{EffortLevel, StepOverride};
use demeteo_hub_protocol::{
    Effort, RunnerRepository, RunnerSettings, RunnerWorkflow, StepOverride as WireOverride,
};

fn steps_json() -> String {
    let step = StepConfig {
        id: "s-build".into(),
        kind: "agent".into(),
        title: "Build".into(),
        ..StepConfig::default()
    };
    serde_json::to_string(&[step]).expect("steps serialise")
}

fn runner() -> RunnerStart {
    RunnerStart {
        source_instance_id: "i-desktop".into(),
        repository: RunnerRepository {
            provider_kind: "github".into(),
            host: "github.com".into(),
            full_name: "acme/app".into(),
        },
        workflow: RunnerWorkflow {
            workflow_id: "w-1".into(),
            version: 3,
            name: "Ship".into(),
            steps_json: steps_json(),
        },
        settings: None,
    }
}

fn start() -> StartFeature {
    StartFeature {
        project_id: "p-1".into(),
        workflow_id: "w-1".into(),
        title: "From the Hub".into(),
        description: "Started from a phone".into(),
        agent_kind: Some("claude-code".into()),
        model: Some("opus".into()),
        effort: Some(Effort::High),
        step_overrides: vec![WireOverride {
            step_id: "s-build".into(),
            agent_kind: Some("codex".into()),
            model: None,
            effort: Some(Effort::Low),
        }],
        max_budget_cents: Some(250),
        max_wall_clock_secs: Some(3_600),
        attachments: Vec::new(),
        runner: Some(Box::new(runner())),
    }
}

fn spec_of(start: &StartFeature) -> Result<RunSpec, RefusalReason> {
    let runner = start.runner.as_deref().expect("a runner start");
    runner_run_spec(start, runner, "f-hub", Vec::new())
}

#[test]
fn a_runner_start_maps_every_field_onto_an_unattended_hub_run() {
    let attachment = RunSpecAttachment {
        staged_path: "spool/a.txt".into(),
        mime: Some("text/plain".into()),
        source_filename: None,
    };
    let start = start();
    let spec = runner_run_spec(&start, &runner(), "f-hub", vec![attachment]).expect("a spec");

    assert_eq!(spec.feature_id.as_deref(), Some("f-hub"));
    assert_eq!(spec.title, "From the Hub");
    assert_eq!(spec.description, "Started from a phone");
    assert_eq!(spec.provider.kind, "github");
    assert_eq!(spec.provider.host, "github.com");
    assert_eq!(spec.repo_path, "acme/app");
    assert_eq!(
        spec.workflow_json,
        serde_json::json!({
            "name": "Ship",
            "description": "",
            "steps": serde_json::from_str::<serde_json::Value>(&steps_json()).expect("json"),
        })
    );
    assert_eq!(spec.agent_kind.as_deref(), Some("claude-code"));
    assert_eq!(spec.model.as_deref(), Some("opus"));
    assert_eq!(spec.effort, Some(EffortLevel::High));
    assert_eq!(
        spec.step_overrides,
        vec![StepOverride {
            step_id: "s-build".into(),
            agent_kind: Some("codex".into()),
            model: None,
            effort: Some(EffortLevel::Low),
        }]
    );
    assert_eq!(spec.attachments.len(), 1);
    assert!(spec.unattended);
    assert!(spec.project_settings.is_none());
    assert_eq!(spec.settings_overlay, None);
    assert_eq!(spec.loop_iterations, None);
    assert_eq!(spec.commit_artifacts, None);
    assert_eq!(spec.origin_to_honour(), Ok(FeatureOrigin::DefaultBranch));
    assert_eq!(spec.launched_by, LaunchedBy::Hub);
}

#[test]
fn a_hub_projects_settings_become_the_overlay_and_the_runs_own_limits() {
    let mut start = start();
    let mut runner = runner();
    runner.settings = Some(RunnerSettings {
        branch_prefix: Some("hub/".into()),
        artifact_subdir: Some("reports/".into()),
        commit_artifacts: Some(true),
        review_entrypoint: Some("/review".into()),
        test_command: Some("cargo test".into()),
        build_command: Some("cargo build".into()),
        coverage_command: Some("cargo llvm-cov".into()),
        conventions_file: Some("AGENTS.md".into()),
        loop_iterations: Some(5),
        sync_resolver_agent_kind: Some("codex".into()),
        sync_resolver_model: Some("gpt-5".into()),
        sync_resolver_effort: Some(Effort::Xhigh),
        sync_review_before_push: Some(false),
    });
    start.runner = Some(Box::new(runner));
    let spec = spec_of(&start).expect("a spec");

    assert_eq!(spec.loop_iterations, Some(5));
    assert_eq!(spec.commit_artifacts, Some(true));
    assert!(
        spec.project_settings.is_none(),
        "an overlay never replaces the detected settings"
    );
    assert_eq!(
        spec.settings_overlay,
        Some(ProjectSettingsOverlay {
            branch_prefix: Some("hub/".into()),
            artifact_subdir: Some("reports/".into()),
            review_entrypoint: Some("/review".into()),
            test_command: Some("cargo test".into()),
            build_command: Some("cargo build".into()),
            coverage_command: Some("cargo llvm-cov".into()),
            conventions_file: Some("AGENTS.md".into()),
            sync_resolver_agent_kind: Some("codex".into()),
            sync_resolver_model: Some("gpt-5".into()),
            sync_resolver_effort: Some(EffortLevel::XHigh),
            sync_review_before_push: Some(false),
        })
    );
}

#[test]
fn the_cap_in_cents_is_the_turn_budget_and_the_hard_cap_with_the_wall_clock() {
    let spec = spec_of(&start()).expect("a spec");
    assert_eq!(spec.max_budget_usd, Some(2.5));
    let budget = spec.budget.expect("a hard cap");
    assert_eq!(budget.max_cost_usd, Some(2.5));
    assert_eq!(budget.max_wall_clock_secs, Some(3_600));

    let mut odd = start();
    odd.max_budget_cents = Some(1_999);
    odd.max_wall_clock_secs = None;
    let spec = spec_of(&odd).expect("a spec");
    assert_eq!(spec.max_budget_usd, Some(19.99));
    let budget = spec.budget.expect("a hard cap");
    assert_eq!(budget.max_cost_usd, Some(19.99));
    assert_eq!(budget.max_wall_clock_secs, None);

    let mut uncapped = start();
    uncapped.max_budget_cents = None;
    uncapped.max_wall_clock_secs = None;
    let spec = spec_of(&uncapped).expect("a spec");
    assert_eq!(spec.max_budget_usd, None);
    assert!(spec.budget.is_none());
}

#[test]
fn steps_a_runner_could_not_ingest_are_refused() {
    let cases: [(&str, String); 4] = [
        ("not json", "{not json".into()),
        ("not a list", r#"{"id":"s-build"}"#.into()),
        ("no steps", "[]".into()),
        (
            "a step missing its kind",
            r#"[{"id":"s-build","title":"Build"}]"#.into(),
        ),
    ];
    for (name, steps) in cases {
        let mut start = start();
        if let Some(runner) = start.runner.as_deref_mut() {
            runner.workflow.steps_json = steps;
        }
        assert_eq!(
            runner_start_refusal(&start),
            Some(RefusalReason::MalformedBody),
            "{name}"
        );
        assert_eq!(
            spec_of(&start).err(),
            Some(RefusalReason::MalformedBody),
            "{name}"
        );
    }
}

#[test]
fn a_repository_or_workflow_a_runner_cannot_use_is_refused() {
    type Edit = fn(&mut RunnerStart);
    let cases: [(&str, Edit); 5] = [
        ("unknown provider", |r| {
            r.repository.provider_kind = "bitbucket".into()
        }),
        ("no host", |r| r.repository.host.clear()),
        ("no repository", |r| r.repository.full_name.clear()),
        ("another workflow", |r| {
            r.workflow.workflow_id = "w-2".into()
        }),
        ("capitalised provider", |r| {
            r.repository.provider_kind = "GitHub".into()
        }),
    ];
    for (name, edit) in cases {
        let mut start = start();
        if let Some(runner) = start.runner.as_deref_mut() {
            edit(runner);
        }
        assert_eq!(
            runner_start_refusal(&start),
            Some(RefusalReason::MalformedBody),
            "{name}"
        );
        assert_eq!(
            spec_of(&start).err(),
            Some(RefusalReason::MalformedBody),
            "{name}"
        );
    }
}

#[test]
fn a_start_with_no_runner_part_is_refused_and_a_whole_one_is_not() {
    let mut bare = start();
    bare.runner = None;
    assert_eq!(
        runner_start_refusal(&bare),
        Some(RefusalReason::MalformedBody)
    );
    assert_eq!(runner_start_refusal(&start()), None);

    let mut gitlab = start();
    if let Some(runner) = gitlab.runner.as_deref_mut() {
        runner.repository.provider_kind = "gitlab".into();
    }
    assert_eq!(runner_start_refusal(&gitlab), None);
}

#[test]
fn a_desktop_refuses_a_start_resolved_for_a_runner() {
    use crate::domain::hub_request::desktop_start_refusal;
    assert_eq!(
        desktop_start_refusal(&start()),
        Some(RefusalReason::MalformedBody)
    );
    let mut bare = start();
    bare.runner = None;
    assert_eq!(desktop_start_refusal(&bare), None);
}
