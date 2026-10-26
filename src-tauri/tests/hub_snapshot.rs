//! The snapshot-builder and snapshot-source tests the harness runs.
//!
//! The harness runs `cd src-tauri && cargo test`, so a test that proves what
//! the Hub snapshot reports has to live here to be seen passing. Nothing in
//! this file needs `demeteo-hub`, which is a Linux-only dev-dependency, so it
//! stays `cfg`-free and runs on every target.

use std::sync::Arc;
use std::time::Duration;

use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::snapshot_source::RepoSnapshotSource;
use demeteo_core::adapters::hub::socket::snapshot_wake::{WakingProjects, WakingWorkflows};
use demeteo_core::adapters::hub::testing::ScriptedSnapshotSource;
use demeteo_core::domain::git_push::without_userinfo;
use demeteo_core::domain::hub_socket::{snapshot, ProjectInput, SnapshotInput};
use demeteo_core::domain::models::{
    EffortLevel, Project, ProjectSettings, ProviderInstance, Repository, Workflow, WorkflowVersion,
};
use demeteo_core::ports::db::{AppSettingsRepository, ProjectRepository, WorkflowRepository};
use demeteo_core::ports::hub::HubSnapshotSource;
use tokio::sync::Notify;

#[test]
fn without_userinfo_drops_user_and_token() {
    assert_eq!(
        without_userinfo("https://user:token@host/repo").as_deref(),
        Some("https://host/repo")
    );
}

#[test]
fn a_token_as_the_username_is_dropped() {
    assert_eq!(
        without_userinfo("https://ghp_x@github.com/o/r").as_deref(),
        Some("https://github.com/o/r")
    );
}

#[test]
fn a_userinfo_of_only_a_password_is_dropped() {
    assert_eq!(
        without_userinfo("https://:tok@host/repo").as_deref(),
        Some("https://host/repo")
    );
}

#[test]
fn a_port_is_kept() {
    assert_eq!(
        without_userinfo("https://user:token@host:8443/repo").as_deref(),
        Some("https://host:8443/repo")
    );
}

#[test]
fn a_url_without_userinfo_is_unchanged() {
    assert_eq!(
        without_userinfo("https://github.com/acme/app").as_deref(),
        Some("https://github.com/acme/app")
    );
}

#[test]
fn dropping_the_userinfo_is_idempotent() {
    let once = without_userinfo("https://user:token@host:8443/repo?x=1");
    let twice = once.as_deref().and_then(without_userinfo);
    assert_eq!(twice, once);
}

#[test]
fn a_remote_that_is_not_http_is_none() {
    assert_eq!(without_userinfo("git@github.com:acme/app.git"), None);
    assert_eq!(without_userinfo("ssh://git@github.com/acme/app.git"), None);
}

fn project(id: &str, name: &str) -> Project {
    Project {
        id: id.into(),
        name: name.to_owned(),
        compute_type: "local".to_owned(),
        remote_host: None,
        status: "active".to_owned(),
        nodes: 0,
        spend: 0.0,
        tokens: 0,
        created_at: 0,
    }
}

fn repository(project_id: &str, provider_id: &str, repo_path: &str) -> Repository {
    Repository {
        id: format!("{project_id}-{repo_path}").into(),
        project_id: project_id.into(),
        provider_id: provider_id.into(),
        repo_path: repo_path.to_owned(),
    }
}

fn provider(id: &str, host: &str) -> ProviderInstance {
    ProviderInstance {
        id: id.into(),
        kind: "github".to_owned(),
        host: host.to_owned(),
        username: "octo-user".to_owned(),
        avatar_url: String::new(),
        created_at: 0,
    }
}

fn settings(project_id: &str, run_shape: serde_json::Value) -> ProjectSettings {
    let mut doc = serde_json::json!({
        "project_id": project_id,
        "worktree_strategy": {
            "default_branch": "main",
            "branch_prefix": "demeteo/",
            "pr_template": null,
        },
        "feature_lifecycle": "manual",
    });
    if let (Some(doc), Some(extra)) = (doc.as_object_mut(), run_shape.as_object()) {
        doc.extend(extra.clone());
    }
    serde_json::from_value(doc).expect("settings fixture parses")
}

fn bare(id: &str, name: &str) -> ProjectInput {
    ProjectInput {
        project: project(id, name),
        settings: None,
        repositories: Vec::new(),
    }
}

fn one_project_on(host: &str) -> SnapshotInput {
    SnapshotInput {
        projects: vec![ProjectInput {
            repositories: vec![repository("p1", "gh", "acme/app")],
            ..bare("p1", "App")
        }],
        workflows: Vec::new(),
        providers: vec![provider("gh", host)],
    }
}

/// Set on the struct, not through `json!`, which turns a non-finite `f64`
/// into `null` and would test `None` instead.
fn budget_cents(usd: f64) -> Option<u64> {
    let mut stored = settings("p1", serde_json::json!({}));
    stored.default_max_budget_usd = Some(usd);
    let input = SnapshotInput {
        projects: vec![ProjectInput {
            settings: Some(stored),
            ..bare("p1", "App")
        }],
        workflows: Vec::new(),
        providers: Vec::new(),
    };
    let mut shot = snapshot(&input);
    let row = shot.run_shape.pop().expect("one run-shape row");
    row.default_max_budget_cents
}

#[test]
fn a_credentialed_provider_host_reports_a_bare_remote_url() {
    let shot = snapshot(&one_project_on("user:token@github.com"));
    assert_eq!(
        shot.projects[0].remote_url.as_deref(),
        Some("https://github.com/acme/app")
    );
    let json = serde_json::to_string(&shot).expect("snapshot serialises");
    assert!(!json.contains("user"), "{json}");
    assert!(!json.contains("token"), "{json}");
}

#[test]
fn a_password_that_ends_the_authority_early_reports_no_remote_url() {
    let shot = snapshot(&one_project_on("u:p/ss@github.com"));
    assert_eq!(shot.projects.len(), 1);
    assert_eq!(shot.projects[0].remote_url, None);
}

#[test]
fn a_missing_repository_or_provider_reports_no_remote_url() {
    let no_repo = SnapshotInput {
        projects: vec![bare("p1", "App")],
        workflows: Vec::new(),
        providers: vec![provider("gh", "github.com")],
    };
    assert_eq!(snapshot(&no_repo).projects[0].remote_url, None);

    let mut no_provider = one_project_on("github.com");
    no_provider.providers.clear();
    assert_eq!(snapshot(&no_provider).projects[0].remote_url, None);

    let mut empty_path = one_project_on("github.com");
    empty_path.projects[0].repositories[0].repo_path.clear();
    assert_eq!(snapshot(&empty_path).projects[0].remote_url, None);
}

#[test]
fn the_first_repository_is_the_one_reported() {
    let input = SnapshotInput {
        projects: vec![ProjectInput {
            repositories: vec![
                repository("p1", "gl", "acme/first"),
                repository("p1", "gh", "acme/second"),
            ],
            ..bare("p1", "App")
        }],
        workflows: Vec::new(),
        providers: vec![provider("gh", "github.com"), provider("gl", "gitlab.com")],
    };
    assert_eq!(
        snapshot(&input).projects[0].remote_url.as_deref(),
        Some("https://gitlab.com/acme/first")
    );
}

#[test]
fn a_dollar_budget_is_reported_in_cents() {
    assert_eq!(budget_cents(12.5), Some(1250));
    assert_eq!(budget_cents(19.99), Some(1999));
    assert_eq!(budget_cents(0.0), Some(0));
}

#[test]
fn a_budget_no_launch_could_honour_is_none() {
    assert_eq!(budget_cents(f64::NAN), None);
    assert_eq!(budget_cents(-1.0), None);
    assert_eq!(budget_cents(f64::INFINITY), None);
}

#[test]
fn every_effort_level_maps_to_its_wire_effort() {
    for level in EffortLevel::ALL {
        let input = SnapshotInput {
            projects: vec![ProjectInput {
                settings: Some(settings(
                    "p1",
                    serde_json::json!({ "default_effort": level }),
                )),
                ..bare("p1", "App")
            }],
            workflows: Vec::new(),
            providers: Vec::new(),
        };
        let shot = snapshot(&input);
        assert_eq!(
            serde_json::to_value(shot.run_shape[0].default_effort).expect("effort serialises"),
            serde_json::to_value(Some(level)).expect("level serialises"),
            "{level:?}"
        );
    }
}

#[test]
fn run_shape_carries_its_fields_verbatim() {
    let input = SnapshotInput {
        projects: vec![ProjectInput {
            settings: Some(settings(
                "p1",
                serde_json::json!({
                    "default_agent_kind": "claude-code",
                    "default_model": "opus",
                    "default_workflow_id": "wf-1",
                    "default_loop_iterations": 4,
                }),
            )),
            ..bare("p1", "App")
        }],
        workflows: Vec::new(),
        providers: Vec::new(),
    };
    let row = &snapshot(&input).run_shape[0];
    assert_eq!(row.project_id, "p1");
    assert_eq!(row.default_agent_kind.as_deref(), Some("claude-code"));
    assert_eq!(row.default_model.as_deref(), Some("opus"));
    assert_eq!(row.default_workflow_id.as_deref(), Some("wf-1"));
    assert_eq!(row.default_loop_iterations, Some(4));
    assert_eq!(row.default_effort, None);
    assert_eq!(row.default_max_budget_cents, None);
}

#[test]
fn a_nul_in_a_project_name_drops_only_that_project() {
    let input = SnapshotInput {
        projects: vec![bare("p1", "Fine"), bare("p2", "Bro\0ken")],
        workflows: Vec::new(),
        providers: Vec::new(),
    };
    let shot = snapshot(&input);
    let ids: Vec<&str> = shot.projects.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["p1"]);
}

#[test]
fn a_nul_in_a_run_shape_field_drops_only_that_row() {
    let input = SnapshotInput {
        projects: vec![
            ProjectInput {
                settings: Some(settings(
                    "p1",
                    serde_json::json!({ "default_model": "op\u{0}us" }),
                )),
                ..bare("p1", "App")
            },
            ProjectInput {
                settings: Some(settings("p2", serde_json::json!({}))),
                ..bare("p2", "Other")
            },
        ],
        workflows: Vec::new(),
        providers: Vec::new(),
    };
    let shot = snapshot(&input);
    assert_eq!(shot.projects.len(), 2);
    let ids: Vec<&str> = shot
        .run_shape
        .iter()
        .map(|r| r.project_id.as_str())
        .collect();
    assert_eq!(ids, ["p2"]);
}

#[test]
fn inputs_in_a_different_order_give_equal_snapshots() {
    let build = |order: [&str; 3]| SnapshotInput {
        projects: order
            .iter()
            .map(|id| ProjectInput {
                settings: Some(settings(id, serde_json::json!({}))),
                repositories: vec![repository(id, "gh", "acme/app")],
                ..bare(id, "App")
            })
            .collect(),
        workflows: Vec::new(),
        providers: vec![provider("gl", "gitlab.com"), provider("gh", "github.com")],
    };
    let forward = snapshot(&build(["a", "b", "c"]));
    let mut reversed = build(["c", "b", "a"]);
    reversed.providers.reverse();
    assert_eq!(forward, snapshot(&reversed));
    let ids: Vec<&str> = forward.projects.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

fn version(workflow_id: &str, steps: serde_json::Value) -> WorkflowVersion {
    WorkflowVersion {
        id: format!("{workflow_id}-v1").as_str().into(),
        workflow_id: workflow_id.into(),
        version: 1,
        steps_json: steps.to_string(),
        definition_json: None,
        note: None,
        created_at: 0,
    }
}

fn only(workflows: Vec<WorkflowVersion>) -> SnapshotInput {
    SnapshotInput {
        projects: Vec::new(),
        workflows,
        providers: Vec::new(),
    }
}

/// AC2. Covers the structured places a local path is stored; a path written
/// into the free text of a command or prompt template is out of scope and
/// crosses scrubbed but intact (docs/HUB.md §11 item 11).
#[test]
fn a_local_path_appears_nowhere_in_the_serialised_snapshot() {
    const PATH: &str = "/home/alice/work/acme";
    let stored = settings(
        "p1",
        serde_json::json!({
            "worktree_strategy": {
                "default_branch": "main",
                "branch_prefix": "demeteo/",
                "pr_template": null,
                "conventions_file": PATH,
                "extra_writable_paths": [PATH],
            },
            "artifact_subdir": PATH,
            "default_model": "opus",
        }),
    );
    let mut workflow = version(
        "wf-1",
        serde_json::json!([{ "id": "build", "cwd": PATH, "command": "make" }]),
    );
    workflow.definition_json = Some(
        serde_json::json!({
            "nodes": [{ "id": "build", "config": { "cwd": PATH, "command": "make" } }],
        })
        .to_string(),
    );
    let input = SnapshotInput {
        projects: vec![ProjectInput {
            settings: Some(stored),
            ..bare("p1", "App")
        }],
        workflows: vec![workflow],
        providers: Vec::new(),
    };

    let shot = snapshot(&input);
    let json = serde_json::to_string(&shot).expect("snapshot serialises");

    assert!(!json.contains(PATH), "{json}");
    assert_eq!(shot.projects.len(), 1, "{json}");
    assert_eq!(shot.run_shape.len(), 1, "{json}");
    assert_eq!(shot.run_shape[0].default_model.as_deref(), Some("opus"));
    assert_eq!(shot.workflows.len(), 1, "{json}");
    assert!(shot.workflows[0].steps_json.contains("make"), "{json}");
    let definition = shot.workflows[0].definition_json.as_deref();
    assert!(definition.is_some_and(|d| d.contains("make")), "{json}");
}

#[test]
fn cwd_is_removed_at_depth_and_the_other_step_keys_survive() {
    let shot = snapshot(&only(vec![version(
        "wf-1",
        serde_json::json!([{
            "id": "outer",
            "kind": "sequence",
            "cwd": "/srv/outer",
            "steps": [{ "id": "inner", "command": "make test", "cwd": "/srv/inner" }],
        }]),
    )]))
    .workflows;
    let steps: serde_json::Value =
        serde_json::from_str(&shot[0].steps_json).expect("steps_json is JSON");
    assert_eq!(
        steps,
        serde_json::json!([{
            "id": "outer",
            "kind": "sequence",
            "steps": [{ "id": "inner", "command": "make test" }],
        }])
    );
}

#[test]
fn an_unparseable_document_omits_only_that_version() {
    let mut broken = version("wf-1", serde_json::json!([]));
    broken.steps_json = "[{".to_owned();
    let mut broken_definition = version("wf-2", serde_json::json!([]));
    broken_definition.definition_json = Some("{nodes".to_owned());
    let shot = snapshot(&only(vec![
        broken,
        broken_definition,
        version("wf-3", serde_json::json!([])),
    ]))
    .workflows;
    let ids: Vec<&str> = shot.iter().map(|w| w.workflow_id.as_str()).collect();
    assert_eq!(ids, ["wf-3"]);
}

#[test]
fn a_token_in_a_step_command_is_masked() {
    let token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let shot = snapshot(&only(vec![version(
        "wf-1",
        serde_json::json!([{ "id": "push", "command": format!("gh auth {token}") }]),
    )]))
    .workflows;
    assert!(
        !shot[0].steps_json.contains(token),
        "{}",
        shot[0].steps_json
    );
    assert!(
        shot[0].steps_json.contains("gh auth"),
        "{}",
        shot[0].steps_json
    );
}

#[test]
fn created_at_is_reported_in_unix_seconds() {
    let mut stored = version("wf-1", serde_json::json!([]));
    stored.created_at = 1_700_000_123_456;
    assert_eq!(
        snapshot(&only(vec![stored])).workflows[0].created_at,
        1_700_000_123
    );
}

#[test]
fn a_version_is_reported_verbatim_and_sorted_by_workflow_id() {
    let mut second = version("wf-b", serde_json::json!([]));
    second.version = 7;
    second.note = Some("tightened gates".to_owned());
    let shot = snapshot(&only(vec![second, version("wf-a", serde_json::json!([]))])).workflows;
    let ids: Vec<&str> = shot.iter().map(|w| w.workflow_id.as_str()).collect();
    assert_eq!(ids, ["wf-a", "wf-b"]);
    assert_eq!(shot[1].id, "wf-b-v1");
    assert_eq!(shot[1].version, 7);
    assert_eq!(shot[1].note.as_deref(), Some("tightened gates"));
    assert_eq!(shot[1].definition_json, None);
}

#[test]
fn a_nul_anywhere_in_a_version_drops_only_that_version() {
    let mut in_note = version("wf-1", serde_json::json!([]));
    in_note.note = Some("no\0te".to_owned());
    let escaped = version("wf-2", serde_json::json!([{ "command": "a\u{0}b" }]));
    let shot = snapshot(&only(vec![
        in_note,
        escaped,
        version("wf-3", serde_json::json!([])),
    ]))
    .workflows;
    let ids: Vec<&str> = shot.iter().map(|w| w.workflow_id.as_str()).collect();
    assert_eq!(ids, ["wf-3"]);
}

#[test]
fn the_repository_source_reports_a_bare_remote_and_only_the_latest_version() {
    let db = Arc::new(
        SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("a scratch DB"))
            .expect("a migrated DB"),
    );
    db.add_provider_instance(provider("gh", "octo:ghp_secret@github.com"))
        .expect("provider saved");
    db.add(project("p1", "App")).expect("project saved");
    db.add_repository(repository("p1", "gh", "acme/app"))
        .expect("repository saved");
    db.save_settings(settings(
        "p1",
        serde_json::json!({ "default_model": "opus" }),
    ))
    .expect("settings saved");
    WorkflowRepository::create(
        &*db,
        Workflow {
            id: "wf-1".into(),
            name: "Ship".to_owned(),
            description: String::new(),
            is_starter: false,
            created_at: 0,
            updated_at: 0,
            schedule: None,
        },
    )
    .expect("workflow saved");
    for n in [1, 2] {
        db.save_version(WorkflowVersion {
            id: format!("wf-1-v{n}").as_str().into(),
            version: n,
            ..version("wf-1", serde_json::json!([]))
        })
        .expect("version saved");
    }

    let shot = RepoSnapshotSource::new(db.clone(), db.clone(), db)
        .snapshot()
        .expect("snapshot built");

    assert_eq!(shot.projects.len(), 1, "{shot:?}");
    assert_eq!(shot.projects[0].id, "p1");
    assert_eq!(
        shot.projects[0].remote_url.as_deref(),
        Some("https://github.com/acme/app")
    );
    assert_eq!(shot.run_shape.len(), 1, "{shot:?}");
    assert_eq!(shot.run_shape[0].default_model.as_deref(), Some("opus"));
    let versions: Vec<(&str, u32)> = shot
        .workflows
        .iter()
        .filter(|w| w.workflow_id == "wf-1")
        .map(|w| (w.id.as_str(), w.version))
        .collect();
    assert_eq!(versions, [("wf-1-v2", 2)]);
}

#[test]
fn an_unscripted_snapshot_source_errs() {
    let source = ScriptedSnapshotSource::default();
    assert_eq!(
        source.snapshot(),
        Err(ScriptedSnapshotSource::UNSCRIPTED.to_owned())
    );
    assert_eq!((source.calls(), source.unscripted()), (1, 1));
}

#[test]
fn a_scripted_snapshot_source_answers_its_script_then_what_was_set() {
    let first = snapshot(&one_project_on("github.com"));
    let set = snapshot(&only(Vec::new()));
    let source = ScriptedSnapshotSource::default();
    source.push(first.clone());
    source.fail("db locked");
    source.set(set.clone());

    assert_eq!(source.snapshot(), Ok(first));
    assert_eq!(source.snapshot(), Err("db locked".to_owned()));
    assert_eq!(source.snapshot(), Ok(set.clone()));
    assert_eq!(source.snapshot(), Ok(set));
    assert_eq!((source.calls(), source.unscripted()), (4, 0));
}

fn scratch_db() -> Arc<SqliteAdapter> {
    Arc::new(
        SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("a scratch DB"))
            .expect("a migrated DB"),
    )
}

fn workflow(id: &str) -> Workflow {
    Workflow {
        id: id.into(),
        name: "Ship".to_owned(),
        description: String::new(),
        is_starter: false,
        created_at: 0,
        updated_at: 0,
        schedule: None,
    }
}

/// Long enough that a permit `notify_one` stored is never missed, short
/// enough that a test proving no permit was stored stays fast: a stored
/// permit completes `notified()` at once, without waiting on the clock.
const NO_WAKE: Duration = Duration::from_millis(50);

async fn woke(wake: &Notify) -> bool {
    tokio::time::timeout(NO_WAKE, wake.notified()).await.is_ok()
}

#[tokio::test]
async fn a_saved_workflow_version_wakes_the_snapshot() {
    let db = scratch_db();
    WorkflowRepository::create(&*db, workflow("wf-1")).expect("workflow saved");
    let workflows = WakingWorkflows::new(db);
    let wake = workflows.wake();

    workflows
        .save_version(version("wf-1", serde_json::json!([])))
        .expect("version saved");

    assert!(woke(&wake).await);
}

#[tokio::test]
async fn saved_project_settings_wake_the_snapshot() {
    let db = scratch_db();
    db.add(project("p1", "App")).expect("project saved");
    let projects = WakingProjects::new(db);
    let wake = projects.wake();

    projects
        .save_settings(settings("p1", serde_json::json!({})))
        .expect("settings saved");

    assert!(woke(&wake).await);
}

#[tokio::test]
async fn a_project_status_change_does_not_wake_the_snapshot() {
    let db = scratch_db();
    db.add(project("p1", "App")).expect("project saved");
    let projects = WakingProjects::new(db);
    let wake = projects.wake();

    projects
        .update_status(&"p1".into(), "archived")
        .expect("status saved");

    assert!(!woke(&wake).await);
}

#[tokio::test]
async fn a_failed_write_does_not_wake_the_snapshot() {
    let db = scratch_db();
    db.add(project("p1", "App")).expect("project saved");
    WorkflowRepository::create(&*db, workflow("wf-1")).expect("workflow saved");
    db.save_version(version("wf-1", serde_json::json!([])))
        .expect("version saved");
    let projects = WakingProjects::new(db.clone());
    let workflows = WakingWorkflows::new(db);
    let (project_wake, workflow_wake) = (projects.wake(), workflows.wake());

    assert!(projects.add(project("p1", "Again")).is_err());
    assert!(workflows
        .save_version(version("wf-1", serde_json::json!([])))
        .is_err());

    assert!(!woke(&project_wake).await);
    assert!(!woke(&workflow_wake).await);
}
