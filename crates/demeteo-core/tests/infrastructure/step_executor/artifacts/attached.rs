use super::*;
use crate::adapters::step_executor::artifacts::resolve_attached_user_attachments;
use crate::domain::artifact::{ArtifactCapture, ArtifactDecl};
use crate::domain::attachment::AttachedFile;
use crate::domain::ids::FeatureId;
use crate::domain::ids::StepExecutionId;
use crate::ports::artifact_store::ArtifactStore;
use std::sync::Arc;

fn step_conf_inline(step_id: &str) -> crate::domain::models::StepConfig {
    crate::domain::models::StepConfig {
        effort: None,
        id: crate::domain::ids::StepId::from(step_id.to_string()),
        kind: "agent".into(),
        title: step_id.into(),
        agent_kind: None,
        model: None,
        prompt_template: None,
        on_failure: None,
        max_iterations: None,
        artifacts: Some(vec![ArtifactDecl {
            name: "report".into(),
            capture: ArtifactCapture::LastWriteTo {
                path: "artifacts/report.md".into(),
            },
            mode: crate::domain::artifact::ArtifactMode::Full,
            inline: true,
        }]),
        verifier: None,
        capability: None,
        allow_network: false,
        allow_shell: false,
        gate_class: None,
        task_list_from: None,
        ..Default::default()
    }
}

#[test]
fn test_resolve_attached_artifacts() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_artifacts_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let path1 = temp_dir.join("s-spec.md");
    std::fs::write(&path1, "This is the spec content.").unwrap();
    let path1_str = path1.to_string_lossy().to_string();

    let path2 = temp_dir.join("s-research.md");
    std::fs::write(&path2, "This is the research content.").unwrap();
    let path2_str = path2.to_string_lossy().to_string();

    let step_execs = vec![
        StepExecution {
            last_failure_fingerprint: None,
            id: StepExecutionId::from("se-1"),
            feature_id: FeatureId::from("f-1"),
            step_id: crate::domain::ids::StepId::from("s-research"),
            step_index: 0,
            step_kind: "agent".to_string(),
            status: "completed".to_string(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            artifact_path: Some(path2_str),
            artifact_paths: vec![],
            error_message: None,
            iteration_count: 0,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            created_at: 0,
            updated_at: 0,
        },
        StepExecution {
            last_failure_fingerprint: None,
            id: StepExecutionId::from("se-2"),
            feature_id: FeatureId::from("f-1"),
            step_id: crate::domain::ids::StepId::from("s-spec"),
            step_index: 1,
            step_kind: "agent".to_string(),
            status: "completed".to_string(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            artifact_path: Some(path1_str),
            artifact_paths: vec![],
            error_message: None,
            iteration_count: 0,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            created_at: 0,
            updated_at: 0,
        },
    ];

    let step_confs = vec![step_conf_inline("s-research"), step_conf_inline("s-spec")];

    let template = "Read the research: [attached — s-research] and the spec: [attached — s-spec]";
    let resolved = resolve_attached_artifacts(template, &step_execs, 1, &*store, &step_confs);
    assert_eq!(
        resolved,
        "=== ATTACHED CONTEXT: s-research (inlined body) ===\nThis is the research content.\n================================\n\n=== ATTACHED CONTEXT: s-spec (inlined body) ===\nThis is the spec content.\n================================\n\nRead the research: [See attached s-research at the beginning of the prompt] and the spec: [See attached s-spec at the beginning of the prompt]"
    );

    let template_prev = "Previous content: [attached — previous step artifact]";
    let resolved_prev =
        resolve_attached_artifacts(template_prev, &step_execs, 1, &*store, &step_confs);
    assert_eq!(
        resolved_prev,
        "=== ATTACHED CONTEXT: s-research (inlined body) ===\nThis is the research content.\n================================\n\nPrevious content: [See attached s-research at the beginning of the prompt]"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// Regression: an *earlier* step whose template references a *later*
/// step's artifact (a "forward reference") must resolve when that later
/// step has already produced an artifact — the exact case that lets the
/// standard pipeline's `s-implement` step read `[attached — s-critic]`
/// on a redirect from the ship gate — and must degrade gracefully to a
/// "not yet generated" note on the first pass, when the later step has
/// not run. This behavior is what makes review-feedback routing work
/// for *any* custom workflow, not just the shipped one, so it is pinned
/// here.
#[test]
fn forward_reference_resolves_on_redirect_and_degrades_on_first_run() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_fwd_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let critic_path = temp_dir.join("critic-review.md");
    std::fs::write(&critic_path, "## Critical Issues\n- fix the thing").unwrap();
    let critic_path_str = critic_path.to_string_lossy().to_string();

    let mk_exec = |id: &str, step_id: &str, idx: u32, artifact: Option<String>| StepExecution {
        last_failure_fingerprint: None,
        id: StepExecutionId::from(id.to_string()),
        feature_id: FeatureId::from("f-1"),
        step_id: crate::domain::ids::StepId::from(step_id.to_string()),
        step_index: idx,
        step_kind: "agent".to_string(),
        status: "completed".to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: artifact,
        artifact_paths: vec![],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        created_at: 0,
        updated_at: 0,
    };

    let step_confs = vec![
        step_conf_inline("s-implement"),
        step_conf_inline("s-critic"),
    ];
    // The implement step (index 0) references the later critic step.
    let template = "Address the review: [attached — s-critic]";

    // Redirect case: s-critic ran on a prior attempt and still holds its
    // artifact (reset_for_redirect only resets the redirect *target*).
    let with_critic = vec![
        mk_exec("se-impl", "s-implement", 0, None),
        mk_exec("se-critic", "s-critic", 1, Some(critic_path_str.clone())),
    ];
    let resolved = resolve_attached_artifacts(template, &with_critic, 0, &*store, &step_confs);
    assert!(
        resolved.contains("ATTACHED CONTEXT: s-critic")
            && resolved.contains("fix the thing")
            && resolved.contains("[See attached s-critic at the beginning of the prompt]"),
        "forward reference to a later step with an artifact should resolve; got:\n{resolved}"
    );

    // First-run case: s-critic has not produced an artifact yet — the
    // placeholder must degrade, not resolve to a stale/empty block.
    let without_critic = vec![
        mk_exec("se-impl", "s-implement", 0, None),
        mk_exec("se-critic", "s-critic", 1, None),
    ];
    let degraded = resolve_attached_artifacts(template, &without_critic, 0, &*store, &step_confs);
    assert!(
        degraded.contains("not found or not yet generated")
            && !degraded.contains("ATTACHED CONTEXT: s-critic"),
        "forward reference with no artifact yet should degrade gracefully; got:\n{degraded}"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_resolve_attached_artifacts_uses_artifact_paths() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_attach_paths_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let artifact_file = temp_dir.join("s-research.md");
    std::fs::write(&artifact_file, "Research content from paths.").unwrap();
    let artifact_str = artifact_file.to_string_lossy().to_string();

    let step_execs = vec![StepExecution {
        last_failure_fingerprint: None,
        id: StepExecutionId::from("se-1"),
        feature_id: FeatureId::from("f-1"),
        step_id: crate::domain::ids::StepId::from("s-research"),
        step_index: 0,
        step_kind: "agent".to_string(),
        status: "completed".to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: None,
        artifact_paths: vec![artifact_str],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        created_at: 0,
        updated_at: 0,
    }];

    let template = "Previous: [attached — previous step artifact]";
    let resolved = resolve_attached_artifacts(
        template,
        &step_execs,
        1,
        &*store,
        &[step_conf_inline("s-research")],
    );
    assert_eq!(
        resolved,
        "=== ATTACHED CONTEXT: s-research (inlined body) ===\nResearch content from paths.\n================================\n\nPrevious: [See attached s-research at the beginning of the prompt]"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_resolve_attached_artifacts_default_uses_path_manifest() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_attach_manifest_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let artifact_file = temp_dir.join("s-research.md");
    std::fs::write(&artifact_file, "Research content.").unwrap();
    let artifact_str = artifact_file.to_string_lossy().to_string();

    let step_execs = vec![StepExecution {
        last_failure_fingerprint: None,
        id: StepExecutionId::from("se-1"),
        feature_id: FeatureId::from("f-1"),
        step_id: crate::domain::ids::StepId::from("s-research"),
        step_index: 0,
        step_kind: "agent".to_string(),
        status: "completed".to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: None,
        artifact_paths: vec![artifact_str.clone()],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        created_at: 0,
        updated_at: 0,
    }];

    let mut conf = step_conf_inline("s-research");
    conf.artifacts.as_mut().unwrap()[0].inline = false;

    let template = "Previous: [attached — previous step artifact]";
    let resolved = resolve_attached_artifacts(template, &step_execs, 1, &*store, &[conf]);
    assert!(
        resolved.contains("(path manifest)"),
        "default mode should emit a path manifest block, got: {}",
        resolved
    );
    assert!(
        resolved.contains(&artifact_str),
        "path manifest should list the on-disk path"
    );
    assert!(
        !resolved.contains("Research content."),
        "path manifest must NOT inline the body"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

// ── resolve_attached_user_attachments fallback footer ───────────────────
//
// The orchestrator stores user-uploaded files on the feature row and
// references them from a prompt via `[attachment — <name>]` placeholders.
// Workflows whose plan/implement templates don't include such a placeholder
// would otherwise leave the agent blind to the attached files — the agent
// has no signal that anything was uploaded. `resolve_attached_user_attachments`
// mitigates this by appending a "User Attached Files" footer when the
// template referenced zero attachments by name.

fn temp_attachment_store() -> (
    crate::adapters::attachment_store::fs::FsAttachmentStore,
    std::path::PathBuf,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "demeteo_attach_fallback_test_{}_{}_{}",
        nanos,
        std::process::id(),
        count
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = crate::adapters::attachment_store::fs::FsAttachmentStore::new(dir.clone());
    (store, dir)
}

fn sample_attachment(name: &str, mime: &str, sha: &str) -> AttachedFile {
    AttachedFile {
        id: format!("at-{}", name),
        name: name.into(),
        mime: mime.into(),
        sha256: sha.into(),
        size: 1024,
        source_filename: name.into(),
    }
}

#[test]
fn user_attachment_footer_added_when_no_placeholder_present() {
    let (store, _dir) = temp_attachment_store();
    let atts = vec![sample_attachment("screenshot.png", "image/png", "abc123")];
    let template = "Plan the change for the new feature."; // no [attachment — …]
    let resolved = resolve_attached_user_attachments(
        template,
        "f-1",
        &atts,
        &store,
        Some("artifacts/_context"),
    );
    assert!(
        resolved.contains("User Attached Files"),
        "expected fallback footer when no placeholder references the attachment, got: {}",
        resolved
    );
    assert!(
        resolved.contains("screenshot.png"),
        "footer should list the attached filename, got: {}",
        resolved
    );
    assert!(
        resolved.contains("abc123.png"),
        "footer should point at the worktree-local file path, got: {}",
        resolved
    );
    // Original prompt preserved verbatim.
    assert!(resolved.contains(template));
}

#[test]
fn user_attachment_footer_omitted_when_placeholder_present() {
    let (store, _dir) = temp_attachment_store();
    let atts = vec![sample_attachment("screenshot.png", "image/png", "abc123")];
    let template = "Describe this image: [attachment — screenshot.png]";
    let resolved = resolve_attached_user_attachments(
        template,
        "f-1",
        &atts,
        &store,
        Some("artifacts/_context"),
    );
    // The placeholder hit produced the standard prepended path-manifest
    // block, not the "not referenced" footer.
    assert!(!resolved.contains("User Attached Files"));
    assert!(resolved.contains("ATTACHED CONTEXT: attachment:screenshot.png"));
    assert!(resolved.contains("abc123.png"));
}

#[test]
fn user_attachment_footer_lists_only_unreferenced() {
    let (store, _dir) = temp_attachment_store();
    let atts = vec![
        sample_attachment("a.png", "image/png", "aaaa"),
        sample_attachment("b.png", "image/png", "bbbb"),
    ];
    let template = "Reference a only: [attachment — a.png]"; // b is not referenced
    let resolved = resolve_attached_user_attachments(
        template,
        "f-1",
        &atts,
        &store,
        Some("artifacts/_context"),
    );
    // a.png is surfaced through the standard prepended block.
    assert!(resolved.contains("ATTACHED CONTEXT: attachment:a.png"));
    // b.png should NOT be in the standard block (no placeholder), but
    // SHOULD appear in the footer.
    assert!(!resolved.contains("ATTACHED CONTEXT: attachment:b.png"));
    assert!(
        resolved.contains("User Attached Files"),
        "footer should fire for the unreferenced attachment, got: {}",
        resolved
    );
    assert!(resolved.contains("b.png"));
    assert!(resolved.contains("bbbb.png"));
}

#[test]
fn user_attachment_noop_when_empty() {
    let (store, _dir) = temp_attachment_store();
    let resolved = resolve_attached_user_attachments(
        "Do the thing.",
        "f-1",
        &[],
        &store,
        Some("artifacts/_context"),
    );
    assert_eq!(resolved, "Do the thing.");
}
