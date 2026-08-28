// Tests extracted from `crates/demeteo-core/src/application/discovery/mod.rs` (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::models::{Project, TITLE_MAX_CHARS};

/// A local project with nothing in it, which is as much as `create` reads.
fn fixture(tag: &str) -> (AppContext, ProjectId) {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-discovery-create-{tag}-{}",
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
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    (ctx, project_id)
}

fn opening(project_id: &ProjectId, title: &str) -> NewDiscovery {
    NewDiscovery {
        project_id: project_id.as_str().to_string(),
        title: title.to_string(),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: None,
        staged_attachments: Vec::new(),
    }
}

/// The title labels the row and is read by no prompt, so a Discovery that has
/// only been opened has had nothing said in it — and the interviewer's first
/// turn is about the first thing the user sends, not about the name they
/// filed it under.
#[tokio::test]
async fn opening_a_discovery_says_nothing_in_it() {
    let (ctx, project_id) = fixture("silent");
    let discovery =
        create(&ctx, opening(&project_id, "  chat + canvas  ")).expect("the discovery opens");

    assert_eq!(discovery.title, "chat + canvas");
    assert!(ctx
        .discoveries
        .list_messages(&discovery.id)
        .expect("the transcript reads back")
        .is_empty());
}

/// The cap is enforced where the row is written, not only where it is typed:
/// the modal's own limit is a courtesy to the user, and every other caller of
/// `discovery_create` reaches this one.
#[tokio::test]
async fn a_name_long_enough_to_be_an_idea_is_refused() {
    let (ctx, project_id) = fixture("capped");
    let idea = "I want to add a chat option so users can ask questions about the project, and \
                generate an interactive canvas for the architecture.";
    assert!(idea.chars().count() > TITLE_MAX_CHARS);

    let refusal =
        create(&ctx, opening(&project_id, idea)).expect_err("a name that long is refused");
    assert!(refusal.contains(&TITLE_MAX_CHARS.to_string()), "{refusal}");

    assert!(create(&ctx, opening(&project_id, "chat + canvas")).is_ok());
}
