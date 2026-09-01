// Tests extracted from `crates/demeteo-core/src/application/ask/pin.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::artifact::Artifact;
use crate::domain::ask_canvas::{canvas_block_shape_example, AskCanvas};
use crate::domain::ids::{MachineId, ProjectId, LOCAL_MACHINE};
use crate::domain::models::{
    AskMessage, AskStatus, AskThread, CanvasPathVerdict, MessageRole, Project,
};
use crate::ports::artifact_store::ArtifactStore;
use std::sync::{Arc, Mutex};

/// An open Ask thread with no project or worktree behind it — `pin_canvas`,
/// `list_pinned` and `export_canvas` only ever read `ctx.ask` and
/// `ctx.artifact_store`, on the same terms [`super::load`] does.
fn fixture(tag: &str) -> (AppContext, AskThreadId) {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-ask-pin-{tag}-{}",
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
        Arc::new(crate::adapters::notification_noop::NoopNotificationAdapter),
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
    let thread = AskThread {
        id: AskThreadId::from(format!("t-{tag}")),
        project_id,
        title: "canvas pin fixture".to_string(),
        status: AskStatus::Open,
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: MachineId::from(LOCAL_MACHINE.to_string()),
        worktree_path: None,
        session_id: None,
        turn_count: 0,
        cost_usd: 0.0,
        tokens: 0,
        network: true,
        created_at: 0,
        updated_at: 0,
    };
    ctx.ask.create(&thread).expect("the thread is stored");
    (ctx, thread.id)
}

/// An assistant message carrying `canvas_block_shape_example`'s canvas, with
/// one path-bearing node (`n2`) recorded unresolved and a commit sha stamped
/// — the shape Acceptance Criterion 1 requires.
fn message_with_canvas(id: &str) -> AskMessage {
    AskMessage {
        id: id.to_string(),
        thread_id: AskThreadId::from("placeholder".to_string()),
        role: MessageRole::Assistant,
        text: format!("Here is the shape.\n\n{}", canvas_block_shape_example()),
        cost_usd: Some(0.01),
        tokens: Some(100),
        turn_activity: None,
        canvas_paths: Some(vec![CanvasPathVerdict {
            node_id: "n2".to_string(),
            path: "git_ops::scope".to_string(),
            resolved: false,
        }]),
        checked_commit_sha: Some("deadbeef".to_string()),
        created_at: 1_700_000_000,
    }
}

/// [`message_with_canvas`] with the canvas's own `title` replaced — the
/// field the pinned list renders. Rebuilt through [`AskCanvas`] rather than
/// by string surgery on the shape example, whose node titles are spelled
/// identically to the canvas title.
fn message_titled(id: &str, title: &str) -> AskMessage {
    let mut canvas: AskCanvas =
        serde_json::from_str(&canvas_block_shape_example()).expect("the shape example parses");
    canvas.title = title.to_string();
    let mut message = message_with_canvas(id);
    message.text = format!(
        "Here is the shape.\n\n{}",
        serde_json::to_string(&canvas).expect("the canvas serializes")
    );
    message
}

fn message_without_canvas(id: &str) -> AskMessage {
    AskMessage {
        id: id.to_string(),
        thread_id: AskThreadId::from("placeholder".to_string()),
        role: MessageRole::Assistant,
        text: "just prose, no canvas here".to_string(),
        cost_usd: Some(0.01),
        tokens: Some(100),
        turn_activity: None,
        canvas_paths: None,
        checked_commit_sha: None,
        created_at: 1_700_000_000,
    }
}

fn append(ctx: &AppContext, thread_id: &AskThreadId, mut message: AskMessage) -> AskMessage {
    message.thread_id = thread_id.clone();
    ctx.ask.append_message(&message).expect("message stores");
    message
}

/// An [`ArtifactStore`] that errors on any call it was not explicitly told
/// to expect, per AGENTS.md §7 — proves `export_canvas` never calls `put`
/// rather than merely asserting no error surfaced.
struct NoPutStore {
    puts: Mutex<Vec<(String, String, String)>>,
}

impl NoPutStore {
    fn new() -> Self {
        Self {
            puts: Mutex::new(Vec::new()),
        }
    }
}

impl ArtifactStore for NoPutStore {
    fn put(&self, feature_id: &str, step_id: &str, artifact: &Artifact) -> Result<String, String> {
        self.puts.lock().unwrap().push((
            feature_id.to_string(),
            step_id.to_string(),
            artifact.name.clone(),
        ));
        Err("NoPutStore: unexpected put".to_string())
    }
    fn get(&self, reference: &str) -> Result<String, String> {
        Err(format!("NoPutStore: unexpected get `{reference}`"))
    }
    fn list_for_step(&self, _feature_id: &str, _step_id: &str) -> Result<Vec<String>, String> {
        Err("NoPutStore: unexpected list_for_step".to_string())
    }
    fn clear_step(&self, _feature_id: &str, _step_id: &str) -> Result<(), String> {
        Err("NoPutStore: unexpected clear_step".to_string())
    }
}

/// Acceptance Criterion 1: pin, read back through `ArtifactStore::get`,
/// deserialize, and the recovered `canvas`, `canvas_paths` (unresolved entry
/// included) and `checked_commit_sha` equal the source message's field for
/// field.
#[tokio::test]
async fn pinning_round_trips_canvas_paths_and_commit_sha() {
    let (ctx, thread_id) = fixture("roundtrip");
    let message = append(&ctx, &thread_id, message_with_canvas("m-1"));

    let reference = pin_canvas(&ctx, &thread_id, &message.id).expect("the pin succeeds");
    let body = ctx
        .artifact_store
        .get(&reference)
        .expect("the pin reads back");
    let snapshot: crate::domain::ask_canvas::PinnedCanvasSnapshot =
        serde_json::from_str(&body).expect("the pin deserializes");

    let expected_turn = crate::domain::ask_canvas::parse_ask_turn(&message.text);
    let expected_canvas = expected_turn
        .canvas
        .expect("the fixture message has a canvas");

    assert_eq!(snapshot.thread_id, thread_id.as_str());
    assert_eq!(snapshot.message_id, message.id);
    assert_eq!(snapshot.canvas, expected_canvas);
    assert_eq!(
        snapshot.canvas_paths,
        message
            .canvas_paths
            .expect("the fixture message has verdicts")
    );
    assert!(snapshot.canvas_paths.iter().any(|v| !v.resolved));
    assert_eq!(snapshot.checked_commit_sha, message.checked_commit_sha);
}

/// Acceptance Criterion 2, application layer: a message with no canvas
/// cannot be pinned.
#[tokio::test]
async fn pinning_a_message_with_no_canvas_is_refused() {
    let (ctx, thread_id) = fixture("no-canvas-pin");
    let message = append(&ctx, &thread_id, message_without_canvas("m-1"));

    let err = pin_canvas(&ctx, &thread_id, &message.id)
        .expect_err("a canvas-free message cannot be pinned");
    assert!(err.contains("no canvas"), "{err}");
}

/// Exporting a message with no canvas is refused on the same terms as
/// pinning it.
#[tokio::test]
async fn exporting_a_message_with_no_canvas_is_refused() {
    let (ctx, thread_id) = fixture("no-canvas-export");
    let message = append(&ctx, &thread_id, message_without_canvas("m-1"));

    let err = export_canvas(&ctx, &thread_id, &message.id)
        .expect_err("a canvas-free message cannot be exported");
    assert!(err.contains("no canvas"), "{err}");
}

/// Re-pinning the same message twice overwrites the same artifact rather
/// than appending a second one, and the second `get` reflects the second
/// pin's content.
#[tokio::test]
async fn repinning_the_same_message_stays_at_one_entry() {
    let (ctx, thread_id) = fixture("repin");
    let message = append(&ctx, &thread_id, message_with_canvas("m-1"));

    let first_ref = pin_canvas(&ctx, &thread_id, &message.id).expect("the first pin succeeds");
    let second_ref = pin_canvas(&ctx, &thread_id, &message.id).expect("the second pin succeeds");
    assert_eq!(first_ref, second_ref, "re-pinning overwrites the same path");

    let pinned = list_pinned(&ctx, &thread_id).expect("the list reads back");
    assert_eq!(pinned.len(), 1);

    let body = ctx
        .artifact_store
        .get(&second_ref)
        .expect("the pin reads back");
    let snapshot: crate::domain::ask_canvas::PinnedCanvasSnapshot =
        serde_json::from_str(&body).expect("the pin deserializes");
    assert_eq!(snapshot.message_id, message.id);
}

/// `export_canvas`'s JSON, parsed, equals what `pin_canvas` would have
/// written for the same message — and exporting never calls `put`.
///
/// Both sides are driven at one `pinned_at`, which is what makes the
/// whole-struct `assert_eq!` possible: sampling the clock twice would leave
/// [`PinnedCanvasSnapshot`](crate::domain::ask_canvas::PinnedCanvasSnapshot)'s
/// derived `PartialEq` unusable across a millisecond boundary.
#[tokio::test]
async fn exporting_matches_a_pin_and_never_writes() {
    const PINNED_AT: i64 = 1_700_000_123_456;

    let (ctx, thread_id) = fixture("export-parity");
    let message = append(&ctx, &thread_id, message_with_canvas("m-1"));

    let reference =
        pin_canvas_at(&ctx, &thread_id, &message.id, PINNED_AT).expect("the pin succeeds");
    let pinned_body = ctx
        .artifact_store
        .get(&reference)
        .expect("the pin reads back");
    let pinned: crate::domain::ask_canvas::PinnedCanvasSnapshot =
        serde_json::from_str(&pinned_body).expect("the pin deserializes");

    let exported_json =
        export_canvas_at(&ctx, &thread_id, &message.id, PINNED_AT).expect("export succeeds");
    let exported: crate::domain::ask_canvas::PinnedCanvasSnapshot =
        serde_json::from_str(&exported_json).expect("the export deserializes");

    assert_eq!(exported, pinned);
    assert_eq!(pinned.pinned_at, PINNED_AT);

    let fake_store: Arc<dyn ArtifactStore> = Arc::new(NoPutStore::new());
    let fake_ctx = AppContext {
        artifact_store: fake_store,
        ..ctx
    };
    export_canvas(&fake_ctx, &thread_id, &message.id)
        .expect("export succeeds against a store that refuses every put");
}

/// A thread with no pins lists empty.
#[tokio::test]
async fn listing_pins_on_an_untouched_thread_is_empty() {
    let (ctx, thread_id) = fixture("empty-list");
    let pinned = list_pinned(&ctx, &thread_id).expect("the list reads back");
    assert_eq!(pinned, Vec::<PinnedCanvasEntry>::new());
}

/// Two pins in one thread come back carrying their own canvas titles and
/// `pinned_at`, which is the only thing that tells them apart: the artifact
/// name is a bare `message_id`, so a list of paths alone renders two rows a
/// reader cannot choose between.
///
/// Watched to fail first against `list_pinned` mapping each reference to
/// `PinnedCanvasEntry { path, title: None, pinned_at: None }` — the shape a
/// bare `list_for_step` return has — where both title assertions went red
/// and no other test moved.
#[tokio::test]
async fn listing_pins_carries_each_canvas_title_and_pinned_at() {
    let (ctx, thread_id) = fixture("titles");
    let first = append(
        &ctx,
        &thread_id,
        message_titled("m-1", "Gate approval flow"),
    );
    let second = append(
        &ctx,
        &thread_id,
        message_titled("m-2", "Worktree lifecycle"),
    );

    let first_ref = pin_canvas_at(&ctx, &thread_id, &first.id, 1_700_000_000_001)
        .expect("the first pin succeeds");
    let second_ref = pin_canvas_at(&ctx, &thread_id, &second.id, 1_700_000_000_002)
        .expect("the second pin succeeds");

    let pinned = list_pinned(&ctx, &thread_id).expect("the list reads back");
    assert_eq!(pinned.len(), 2);

    let by_path = |reference: &str| {
        pinned
            .iter()
            .find(|entry| entry.path == reference)
            .unwrap_or_else(|| panic!("the list holds an entry for {reference}"))
            .clone()
    };
    assert_eq!(
        by_path(&first_ref).title.as_deref(),
        Some("Gate approval flow")
    );
    assert_eq!(by_path(&first_ref).pinned_at, Some(1_700_000_000_001));
    assert_eq!(
        by_path(&second_ref).title.as_deref(),
        Some("Worktree lifecycle")
    );
    assert_eq!(by_path(&second_ref).pinned_at, Some(1_700_000_000_002));
}

/// A file in the scope directory that is not a readable snapshot degrades to
/// a path-only entry. One corrupt pin must not make every other pin in the
/// thread unreachable, which is what returning `Err` for the whole list
/// would do.
///
/// Watched to fail first against a `list_pinned` that propagated
/// `serde_json::from_str`'s error with `?`: `expect("the list reads back")`
/// panicked on the malformed body and only this test went red.
#[tokio::test]
async fn a_malformed_entry_degrades_to_a_path_only_row() {
    let (ctx, thread_id) = fixture("malformed");
    let message = append(&ctx, &thread_id, message_titled("m-1", "Still readable"));
    let good_ref = pin_canvas(&ctx, &thread_id, &message.id).expect("the pin succeeds");

    // Derived from the reference the store just handed back rather than from
    // the on-disk layout, which is the adapter's to change.
    let scope_dir = std::path::Path::new(&good_ref)
        .parent()
        .expect("a stored pin has a parent directory");
    let broken = scope_dir.join("m-0.canvas.json");
    std::fs::write(&broken, "{ this is not a snapshot").expect("the broken entry writes");

    let pinned = list_pinned(&ctx, &thread_id).expect("the list reads back");
    assert_eq!(pinned.len(), 2);

    let good = pinned
        .iter()
        .find(|entry| entry.path == good_ref)
        .expect("the readable pin is still listed");
    assert_eq!(good.title.as_deref(), Some("Still readable"));

    let degraded = pinned
        .iter()
        .find(|entry| entry.path != good_ref)
        .expect("the malformed entry is still listed");
    assert_eq!(degraded.path, broken.to_string_lossy());
    assert_eq!(degraded.title, None);
    assert_eq!(degraded.pinned_at, None);
}
