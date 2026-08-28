// Tests extracted from `crates/demeteo-core/src/application/discovery/mod.rs` (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::models::Project;

const IDEA: &str = "I want a chat option that renders an architecture canvas.";

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
            name: "seed fixture".to_string(),
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

fn opened(ctx: &AppContext, project_id: &ProjectId, title: &str) -> Discovery {
    create(
        ctx,
        NewDiscovery {
            project_id: project_id.as_str().to_string(),
            title: title.to_string(),
            agent_kind: "claude-code".to_string(),
            model: None,
            effort: None,
            machine_id: None,
            staged_attachments: Vec::new(),
        },
    )
    .expect("the discovery opens")
}

#[tokio::test]
async fn the_idea_a_discovery_is_opened_on_is_the_first_thing_said_in_it() {
    let (ctx, project_id) = fixture("stored");
    let discovery = opened(&ctx, &project_id, &format!("  {IDEA}  "));

    let messages = ctx
        .discoveries
        .list_messages(&discovery.id)
        .expect("the transcript reads back");
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, IDEA);
}

/// The assertion the storage one exists for: a seed the prompt does not carry
/// is a seed the interviewer never sees, and the first turn always re-seeds
/// because no harness holds a session for a Discovery that has not taken one.
#[tokio::test]
async fn the_seed_is_in_the_prompt_the_first_turn_sends() {
    let (ctx, project_id) = fixture("prompted");
    let discovery = opened(&ctx, &project_id, IDEA);
    let transcript = ctx
        .discoveries
        .list_messages(&discovery.id)
        .expect("the transcript reads back");

    let prompt = question::render_turn_prompt(question::TurnPrompt {
        reseed: true,
        context: "",
        transcript: &transcript,
        attachments: &[],
        reads_images: true,
        user_text: "lets start",
    });

    assert!(prompt.contains(IDEA), "{prompt}");
}
