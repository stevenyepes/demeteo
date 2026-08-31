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
        network: true,
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

/// The modal's network default reaches the row rather than being overridden
/// by a literal: a thread opened with the network off is persisted off, so
/// its first turn cannot run with `Access::Allow`.
#[tokio::test]
async fn creating_a_thread_with_the_network_off_persists_it_off() {
    let (ctx, project_id) = fixture("network-off", "local", None);

    let thread = create(
        &ctx,
        NewAskThread {
            network: false,
            ..opening(&project_id, "offline question", None)
        },
    )
    .expect("the thread opens");

    assert!(!thread.network);
    let stored = ctx
        .ask
        .get(&thread.id)
        .expect("the thread reads back")
        .expect("the thread was persisted");
    assert!(!stored.network);
}

/// A caller that never names `network` keeps the posture that predates the
/// control, so nothing built against the old shape changes behaviour.
#[tokio::test]
async fn omitting_the_network_field_keeps_the_thread_on() {
    let (ctx, project_id) = fixture("network-default", "local", None);
    let opening: NewAskThread = serde_json::from_value(serde_json::json!({
        "project_id": project_id.as_str(),
        "title": "quick question",
        "agent_kind": "claude-code",
    }))
    .expect("an opening with no network key deserializes");

    let thread = create(&ctx, opening).expect("the thread opens");

    assert!(thread.network);
    let stored = ctx.ask.get(&thread.id).unwrap().unwrap();
    assert!(stored.network);
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
            canvas_paths: None,
            checked_commit_sha: None,
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
            canvas_paths: None,
            checked_commit_sha: None,
            created_at: thread.created_at + 1,
        })
        .unwrap();

    let detail = load(&ctx, &thread.id).expect("the thread loads");
    assert_eq!(detail.thread.id, thread.id);
    let ids: Vec<&str> = detail
        .messages
        .iter()
        .map(|m| m.message.id.as_str())
        .collect();
    assert_eq!(ids, vec!["m-1", "m-2"]);
}

/// An assistant message ending in a valid canvas block is derived into a
/// view whose `prose` has the block cut out and whose `canvas` is parsed.
#[tokio::test]
async fn load_derives_prose_and_canvas_from_an_assistant_message() {
    let (ctx, project_id) = fixture("canvas", "local", None);
    let thread = create(&ctx, opening(&project_id, "quick question", None)).unwrap();
    let text = format!(
        "Here is the shape.\n\n{}",
        crate::domain::ask_canvas::canvas_block_shape_example()
    );
    ctx.ask
        .append_message(&AskMessage {
            id: "m-1".to_string(),
            thread_id: thread.id.clone(),
            role: crate::domain::models::MessageRole::Assistant,
            text,
            cost_usd: None,
            tokens: None,
            turn_activity: None,
            canvas_paths: None,
            checked_commit_sha: None,
            created_at: thread.created_at,
        })
        .unwrap();

    let detail = load(&ctx, &thread.id).expect("the thread loads");
    let view = &detail.messages[0];
    assert_eq!(view.turn.prose, "Here is the shape.");
    assert!(view.turn.canvas.is_some());
    assert_eq!(view.turn.canvas_error, None);
}

/// An assistant message with no canvas block derives to `canvas: None` and a
/// `prose` equal to the trimmed message text.
#[tokio::test]
async fn load_derives_prose_only_when_there_is_no_canvas_block() {
    let (ctx, project_id) = fixture("no-canvas", "local", None);
    let thread = create(&ctx, opening(&project_id, "quick question", None)).unwrap();
    ctx.ask
        .append_message(&AskMessage {
            id: "m-1".to_string(),
            thread_id: thread.id.clone(),
            role: crate::domain::models::MessageRole::Assistant,
            text: "  just prose, nothing else  ".to_string(),
            cost_usd: None,
            tokens: None,
            turn_activity: None,
            canvas_paths: None,
            checked_commit_sha: None,
            created_at: thread.created_at,
        })
        .unwrap();

    let detail = load(&ctx, &thread.id).expect("the thread loads");
    let view = &detail.messages[0];
    assert_eq!(view.turn.prose, "just prose, nothing else");
    assert_eq!(view.turn.canvas, None);
    assert_eq!(view.turn.canvas_error, None);
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

/// `update_settings` changes model, effort, and network on the stored
/// thread, and leaves `agent_kind` untouched — a thread's harness is fixed at
/// creation and `AskThreadPatch` has no field for it.
#[tokio::test]
async fn update_settings_changes_model_effort_and_network_only() {
    let (ctx, project_id) = fixture("settings", "local", None);
    let thread = create(&ctx, opening(&project_id, "quick question", None)).unwrap();
    assert_eq!(thread.model, None);
    assert_eq!(thread.effort, None);
    assert!(thread.network);

    let updated = update_settings(
        &ctx,
        &thread.id,
        AskThreadPatch {
            model: Some(Some("claude-opus-5".to_string())),
            effort: Some(Some(crate::domain::models::EffortLevel::High)),
            network: Some(false),
            ..Default::default()
        },
    )
    .expect("settings update succeeds");

    assert_eq!(updated.model.as_deref(), Some("claude-opus-5"));
    assert_eq!(
        updated.effort,
        Some(crate::domain::models::EffortLevel::High)
    );
    assert!(!updated.network);
    assert_eq!(updated.agent_kind, thread.agent_kind);

    let stored = ctx.ask.get(&thread.id).unwrap().unwrap();
    assert_eq!(stored.model.as_deref(), Some("claude-opus-5"));
    assert_eq!(
        stored.effort,
        Some(crate::domain::models::EffortLevel::High)
    );
    assert!(!stored.network);
    assert_eq!(stored.agent_kind, thread.agent_kind);
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

/// What a surface that mounted mid-turn reads: the claim, and only for as
/// long as it is held.
#[tokio::test]
async fn turn_running_tracks_the_claim() {
    let (ctx, project_id) = fixture("running", "local", None);
    let thread =
        create(&ctx, opening(&project_id, "quick question", None)).expect("the thread opens");

    assert!(!turn_running(&ctx, &thread.id));

    let claim = ctx
        .ask_turns
        .clone()
        .try_claim(thread.id.as_str())
        .expect("the first turn claims the thread");
    assert!(turn_running(&ctx, &thread.id));

    drop(claim);
    assert!(!turn_running(&ctx, &thread.id));
}
