// Tests extracted from `crates/demeteo-core/src/application/ask/mod.rs` (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::LOCAL_MACHINE;
use crate::domain::models::{Machine, Project, TITLE_MAX_CHARS};

/// A project with nothing in it, which is as much as `create` reads, plus
/// whatever machine the case under test needs configured.
fn fixture(tag: &str, compute_type: &str, remote_host: Option<&str>) -> (AppContext, ProjectId) {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-ask-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let project_id = ProjectId::from(format!("p-{tag}"));
    ctx.projects
        .add(Project {
            id: project_id.clone(),
            name: "name fixture".to_string(),
            compute_type: compute_type.to_string(),
            remote_host: remote_host.map(|h| MachineId::from(h.to_string())),
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    (ctx, project_id)
}

fn add_machine(ctx: &AppContext, id: &str) {
    ctx.machines
        .add(Machine {
            id: MachineId::from(id.to_string()),
            name: id.to_string(),
            host: "example.internal".to_string(),
            port: 22,
            username: "demeteo".to_string(),
            auth_type: "key".to_string(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: Some(false),
            setup_commands: None,
            notify_webhook_url: None,
        })
        .expect("the machine is stored");
}

fn opening(project_id: &ProjectId, title: &str, machine_id: Option<&str>) -> NewAskThread {
    NewAskThread {
        project_id: project_id.as_str().to_string(),
        title: title.to_string(),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: machine_id.map(str::to_string),
    }
}

/// §4.5's rule reaches Ask through the same call: a value the user gave is
/// never overridden, even on a project that would otherwise resolve
/// somewhere else.
#[tokio::test]
async fn an_explicit_machine_choice_is_preserved() {
    let (ctx, project_id) = fixture("explicit", "local", None);
    add_machine(&ctx, "rig-1");

    let thread = create(&ctx, opening(&project_id, "quick question", Some("rig-1")))
        .expect("the thread opens");

    assert_eq!(thread.machine_id.as_str(), "rig-1");
}

/// No choice on a local project takes the desktop host, which needs no
/// `machines` row (V38) to be accepted.
#[tokio::test]
async fn a_local_project_with_no_choice_uses_the_local_machine() {
    let (ctx, project_id) = fixture("local", "local", None);

    let thread =
        create(&ctx, opening(&project_id, "quick question", None)).expect("the thread opens");

    assert_eq!(thread.machine_id.as_str(), LOCAL_MACHINE);
}

/// No choice on a remote project takes the project's own configured host —
/// where Demeteo cloned the repository.
#[tokio::test]
async fn a_remote_project_with_no_choice_uses_its_configured_host() {
    let (ctx, project_id) = fixture("remote", "remote", Some("rig-2"));
    add_machine(&ctx, "rig-2");

    let thread =
        create(&ctx, opening(&project_id, "quick question", None)).expect("the thread opens");

    assert_eq!(thread.machine_id.as_str(), "rig-2");
}

/// A machine nothing is configured for is refused rather than silently
/// accepted, whether it came from the picker or the project's own host.
#[tokio::test]
async fn an_unconfigured_machine_is_refused() {
    let (ctx, project_id) = fixture("unconfigured", "local", None);

    let refusal = create(&ctx, opening(&project_id, "quick question", Some("ghost")))
        .expect_err("an unconfigured machine is refused");
    assert!(refusal.contains("ghost"), "{refusal}");
}

/// Creation validates the title, starts `open` with zero roll-up telemetry,
/// and persists what it returns.
#[tokio::test]
async fn creating_a_thread_starts_open_with_zero_telemetry() {
    let (ctx, project_id) = fixture("zeroed", "local", None);

    let thread =
        create(&ctx, opening(&project_id, "  spend estimate  ", None)).expect("the thread opens");

    assert_eq!(thread.title, "spend estimate");
    assert_eq!(thread.status, AskStatus::Open);
    assert_eq!(thread.turn_count, 0);
    assert_eq!(thread.cost_usd, 0.0);
    assert_eq!(thread.tokens, 0);

    let stored = ctx
        .ask
        .get(&thread.id)
        .expect("the thread reads back")
        .expect("the thread was persisted");
    assert_eq!(stored.title, "spend estimate");
}

/// The cap is enforced where the row is written, the same terms
/// `discovery::create` enforces it on.
#[tokio::test]
async fn a_title_long_enough_to_be_an_idea_is_refused() {
    let (ctx, project_id) = fixture("capped", "local", None);
    let idea = "I want to add a chat option so users can ask questions about the project, and \
                generate an interactive canvas for the architecture.";
    assert!(idea.chars().count() > TITLE_MAX_CHARS);

    let refusal =
        create(&ctx, opening(&project_id, idea, None)).expect_err("a name that long is refused");
    assert!(refusal.contains(&TITLE_MAX_CHARS.to_string()), "{refusal}");
}

/// A project's Ask threads list only its own, most recently touched first.
#[tokio::test]
async fn listing_scopes_to_the_project_newest_first() {
    let (ctx, project_id) = fixture("listing", "local", None);
    let other_project_id = ProjectId::from("p-listing-other".to_string());
    ctx.projects
        .add(Project {
            id: other_project_id.clone(),
            name: "other project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the other project is stored");

    let first = create(&ctx, opening(&project_id, "first", None)).unwrap();
    let second = create(&ctx, opening(&project_id, "second", None)).unwrap();
    create(&ctx, opening(&other_project_id, "elsewhere", None)).unwrap();

    ctx.ask
        .update(
            &second.id,
            &AskThreadPatch::default(),
            second.updated_at + 1000,
        )
        .expect("the second thread is touched later");

    let listed = list_for_project(&ctx, project_id.as_str()).expect("the list reads back");
    let ids: Vec<&str> = listed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec![second.id.as_str(), first.id.as_str()]);
}

/// Load returns the whole transcript, in the order it was appended.
#[tokio::test]
async fn load_returns_the_thread_and_its_transcript() {
    let (ctx, project_id) = fixture("load", "local", None);
    let thread = create(&ctx, opening(&project_id, "quick question", None)).unwrap();

    ctx.ask
        .append_message(&AskMessage {
            id: "m-1".to_string(),
            thread_id: thread.id.clone(),
            role: crate::domain::models::MessageRole::User,
            text: "how does the executor work?".to_string(),
            cost_usd: None,
            tokens: None,
            turn_activity: None,
            created_at: thread.created_at,
        })
        .unwrap();
    ctx.ask
        .append_message(&AskMessage {
            id: "m-2".to_string(),
            thread_id: thread.id.clone(),
            role: crate::domain::models::MessageRole::Assistant,
            text: "here is how".to_string(),
            cost_usd: Some(0.02),
            tokens: Some(512),
            turn_activity: None,
            created_at: thread.created_at + 1,
        })
        .unwrap();

    let detail = load(&ctx, &thread.id).expect("the thread loads");
    assert_eq!(detail.thread.id, thread.id);
    let ids: Vec<&str> = detail.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["m-1", "m-2"]);
}

/// Renaming updates and returns the new title and advances `updated_at`.
#[tokio::test]
async fn renaming_updates_the_title_and_advances_updated_at() {
    let (ctx, project_id) = fixture("rename", "local", None);
    let thread = create(&ctx, opening(&project_id, "quick question", None)).unwrap();

    let renamed = rename(&ctx, &thread.id, "  a better name  ").expect("the rename succeeds");

    assert_eq!(renamed.title, "a better name");
    assert!(renamed.updated_at >= thread.updated_at);
    let stored = ctx.ask.get(&thread.id).unwrap().unwrap();
    assert_eq!(stored.title, "a better name");
}

/// Deleting a thread makes a later load fail.
#[tokio::test]
async fn deleting_a_thread_makes_it_unloadable() {
    let (ctx, project_id) = fixture("delete", "local", None);
    let thread = create(&ctx, opening(&project_id, "quick question", None)).unwrap();

    delete(&ctx, &thread.id).expect("the thread deletes");

    assert!(load(&ctx, &thread.id).is_err());
}

/// Load, rename and delete all reject a thread nothing created.
#[tokio::test]
async fn missing_threads_are_rejected_with_a_clear_error() {
    let (ctx, _project_id) = fixture("missing", "local", None);
    let ghost = AskThreadId::from("no-such-thread".to_string());

    let load_err = load(&ctx, &ghost).expect_err("load rejects a missing thread");
    assert!(load_err.contains("no-such-thread"), "{load_err}");

    let rename_err = rename(&ctx, &ghost, "new name").expect_err("rename rejects a missing thread");
    assert!(rename_err.contains("no-such-thread"), "{rename_err}");

    let delete_err = delete(&ctx, &ghost).expect_err("delete rejects a missing thread");
    assert!(delete_err.contains("no-such-thread"), "{delete_err}");
}
