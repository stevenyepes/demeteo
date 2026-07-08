use super::*;
use crate::adapters::step_executor::artifacts::resolve_attached_user_attachments;
use crate::domain::attachment::AttachedFile;
use crate::domain::ids::FeatureId;
use crate::domain::ids::StepExecutionId;
use crate::ports::artifact_store::ArtifactStore;
use std::sync::Arc;

fn step_conf_inline(step_id: &str) -> crate::domain::models::StepConfig {
    crate::domain::models::StepConfig {
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

#[test]
fn test_inject_artifact_contract_empty() {
    let prompt = "Do the thing.";
    let result = inject_artifact_contract(prompt, None);
    assert_eq!(result, prompt);

    let result = inject_artifact_contract(prompt, Some(&[]));
    assert_eq!(result, prompt);
}

#[test]
fn test_inject_artifact_contract_with_decls() {
    let prompt = "Write the spec.";
    let decls = vec![ArtifactDecl::full_path("spec", "docs/spec.md")];
    let result = inject_artifact_contract(prompt, Some(&decls));
    assert!(result.contains("## Expected Artifacts (orchestrator contract)"));
    assert!(result.contains("Write `docs/spec.md`"));
    assert!(result.contains("artifact `spec`"));
    assert!(result.starts_with("Write the spec."));
}

#[test]
fn test_inject_artifact_contract_all_capture_kinds() {
    use crate::domain::artifact::DiffBase;
    let prompt = "Implement everything.";
    let decls = vec![
        ArtifactDecl::full_path("spec", "docs/spec.md"),
        ArtifactDecl {
            name: "impl".into(),
            capture: ArtifactCapture::AllWrites,
            mode: crate::domain::artifact::ArtifactMode::Full,
            inline: false,
        },
        ArtifactDecl {
            name: "diff".into(),
            capture: ArtifactCapture::Diff {
                base: DiffBase::WorktreeBase,
                path_filter: None,
            },
            mode: crate::domain::artifact::ArtifactMode::Full,
            inline: false,
        },
        ArtifactDecl {
            name: "wt".into(),
            capture: ArtifactCapture::Worktree {
                path: Some("src/".into()),
            },
            mode: crate::domain::artifact::ArtifactMode::None,
            inline: false,
        },
    ];
    let result = inject_artifact_contract(prompt, Some(&decls));
    assert!(result.contains("Write `docs/spec.md`"));
    assert!(result.contains("Every file you write will be captured"));
    assert!(result.contains("A diff will be computed"));
    assert!(result.contains("Worktree pointer for `src/`"));
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

// ── inject_operating_boundary ────────────────────────────────────────────

use crate::domain::permission::{resolve_profile, PermissionProfile, StepCapability};

#[test]
fn boundary_implement_emits_positive_preamble_with_full_access() {
    // Implement steps historically got no boundary block at all
    // (`return prompt.to_string()`). That left the agent without a
    // positive signal that it can write anywhere in the worktree —
    // it had to infer "no boundary = full access" from the absence
    // of a restriction, and the inferred default was often wrong for
    // agents that had just read a restrictive boundary (e.g. the
    // ANALYSIS mode in s-survey). The boundary now emits an
    // explicit IMPLEMENT preamble that names the no-separate-report-
    // folder rule and the commit-vs-untracked contract for the
    // report subdir, so agents carry over the right model between
    // adjacent steps in a workflow.
    let prompt = "do the work";
    let out = inject_operating_boundary(
        prompt,
        StepCapability::Implement,
        &PermissionProfile::all_allow(),
    );
    assert!(
        out.contains("IMPLEMENT mode"),
        "Implement steps now get an explicit positive preamble, got: {out}"
    );
    assert!(
        out.contains("full read/write access"),
        "preamble must declare full read/write access, got: {out}"
    );
    assert!(
        out.contains("no separate report folder") || out.contains("no separate \"report\" folder"),
        "preamble must clarify there's no separate report folder for Implement steps, got: {out}"
    );
    // Original prompt is preserved after the block.
    assert!(out.contains("do the work"));
    // Block comes first.
    assert!(
        out.find("Operating Boundary").unwrap() < out.find("do the work").unwrap(),
        "IMPLEMENT boundary must be prepended, not appended"
    );
}

#[test]
fn boundary_read_only_forbids_writes_shell_and_network() {
    let p = resolve_profile(StepCapability::ReadOnly, false, false);
    let out = inject_operating_boundary("review this", StepCapability::ReadOnly, &p);
    assert!(out.contains("REVIEW-ONLY mode"));
    assert!(out.contains("MUST NOT create, edit"));
    assert!(out.contains("MUST NOT run shell commands."));
    assert!(out.contains("MUST NOT access the network."));
    // The original prompt is preserved after the block.
    assert!(out.contains("review this"));
    // Block comes first.
    assert!(out.find("Operating Boundary").unwrap() < out.find("review this").unwrap());
}

#[test]
fn boundary_artifacts_scopes_writes_and_blocks_implementation() {
    let p = resolve_profile(StepCapability::Artifacts, false, false);
    let out = inject_operating_boundary("write the spec", StepCapability::Artifacts, &p);
    assert!(out.contains("ANALYSIS mode"));
    assert!(out.contains("ONLY write files under the `artifacts/` directory."));
    assert!(out.contains("do NOT make them"));
    assert!(out.contains("MUST NOT run shell commands."));
}

#[test]
fn boundary_verify_allows_shell_but_forbids_source_edits() {
    let p = resolve_profile(StepCapability::Verify, false, false);
    let out = inject_operating_boundary("validate", StepCapability::Verify, &p);
    assert!(out.contains("VALIDATION mode"));
    assert!(out.contains("run build/test/lint/audit commands"));
    assert!(out.contains("MUST NOT fix or modify source code."));
    // Verify has shell, so no "MUST NOT run shell" line.
    assert!(!out.contains("MUST NOT run shell commands."));
}

#[test]
fn boundary_reflects_allow_network_override() {
    let p = resolve_profile(StepCapability::Artifacts, true, false);
    let out = inject_operating_boundary("research", StepCapability::Artifacts, &p);
    assert!(out.contains("MAY use web search/fetch"));
    assert!(!out.contains("MUST NOT access the network."));
}

#[test]
fn boundary_reflects_allow_shell_override() {
    let p = resolve_profile(StepCapability::Artifacts, false, true);
    let out = inject_operating_boundary("research with git log", StepCapability::Artifacts, &p);
    // Shell widened on → no shell prohibition.
    assert!(!out.contains("MUST NOT run shell commands."));
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

// ── materialize_external_artifact_paths (remote-machine regression) ─────
//
// The previous implementation used `std::fs::copy` /
// `std::fs::create_dir_all` unconditionally, which silently dropped
// bytes for remote steps: the worktree path string pointed at a
// directory on the SSH target, not on the Tauri host, so the local
// `std::fs` calls failed (or wrote a phantom file to a path that
// didn't exist remotely), and the opencode agent on the remote box
// ended up with a prompt pointing at a file it couldn't `Read` under
// its `external_directory: deny` fence.
//
// The fix routes the write through `ExecutionPort::write_file`, which
// dispatches SFTP for remote and `std::fs` for local. These tests
// pin both halves of that contract:

use std::sync::Mutex;

/// `ExecutionPort` mock that records every `write_file` /
/// `write_file_bytes` / `get_metadata` call so the test can assert
/// the artifact ended up on the *target* host (path string) — not on
/// the Tauri host. Everything else is a benign no-op.
struct RecordingExec {
    writes: Mutex<Vec<(String, String, String)>>, // (machine_id, path, content)
    metadata_results: Mutex<std::collections::HashMap<String, crate::ports::execution::SftpEntry>>,
}

impl RecordingExec {
    fn new() -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            metadata_results: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn write_count(&self) -> usize {
        self.writes.lock().unwrap().len()
    }

    fn recorded_writes(&self) -> Vec<(String, String, String)> {
        self.writes.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::ports::execution::ExecutionPort for RecordingExec {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        self.writes.lock().unwrap().push((
            machine_id.to_string(),
            path.to_string(),
            content.to_string(),
        ));
        Ok(())
    }
    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let s = String::from_utf8_lossy(content).to_string();
        self.writes
            .lock()
            .unwrap()
            .push((machine_id.to_string(), path.to_string(), s));
        Ok(())
    }
    async fn get_metadata(
        &self,
        _: &str,
        path: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        self.metadata_results
            .lock()
            .unwrap()
            .remove(path)
            .ok_or_else(|| format!("not found: {}", path))
    }
    async fn list_dir(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        Ok(vec![])
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        Ok("/tmp".to_string())
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Ok("test".to_string())
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("control_rpc not supported by RecordingExec".to_string())
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("RecordingExec: spawn_interactive not supported".to_string())
    }
}

fn temp_artifact(name: &str, body: &str) -> (tempdir::TempDir, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "demeteo_materialize_test_{}_{}_{}",
        nanos,
        std::process::id(),
        count
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, body).unwrap();
    (tempdir::TempDir::from_path(dir.clone()), file)
}

#[tokio::test]
async fn materialize_external_paths_writes_to_remote_worktree_via_exec() {
    // Source artifact (always local — the FS artifact store lives on
    // the Tauri host).
    let (_src_dir, src_path) = temp_artifact("implementation-plan.md", "# Plan body\n");
    let src_str = src_path.to_string_lossy().to_string();

    // Target worktree path. This path is on the REMOTE machine: it
    // must NOT be touched on the local host. The previous
    // implementation called `std::fs::create_dir_all` on this string,
    // which silently failed (or wrote a phantom local file).
    let remote_wt = "/home/builder/.demeteo/projects/myrepo/myrepo_wt_f-abc-step-s-implement";

    let exec = RecordingExec::new();
    let prompt = format!(
        "=== ATTACHED CONTEXT: s-plan (path manifest) ===\n\
         The following artifacts from step `s-plan` are on disk:\n\n\
         - `{src}`\n\n\
         Use your Read tool to load them on demand...\n================================\n\n\
         You are an implementation engineer...",
        src = src_str
    );

    let rewritten =
        materialize_external_artifact_paths(&prompt, remote_wt, &exec, "m-builder").await;

    // The remote worktree's _context dir got exactly one write,
    // routed via the exec port to the remote machine_id.
    assert_eq!(
        exec.write_count(),
        1,
        "exactly one write expected; got {:?}",
        exec.recorded_writes()
    );
    let (machine_id, dest_path, content) = &exec.recorded_writes()[0];
    assert_eq!(
        machine_id, "m-builder",
        "write must target the remote machine"
    );
    assert!(
        dest_path.starts_with(remote_wt),
        "destination must live under the remote worktree, got {dest_path}"
    );
    assert!(
        dest_path.ends_with("/artifacts/_context/implementation-plan.md"),
        "destination must be the canonical _context/ copy, got {dest_path}"
    );
    assert_eq!(content, "# Plan body\n", "file body must round-trip");

    // The prompt was rewritten to point at the new path so the
    // opencode Read tool finds the file inside the worktree.
    assert!(
        rewritten.contains(dest_path),
        "rewritten prompt must reference the new path; got: {rewritten}"
    );
    assert!(
        !rewritten.contains(&src_str),
        "old local path must be replaced; got: {rewritten}"
    );

    // The phantom local file MUST NOT exist at the remote path string.
    assert!(
        !std::path::Path::new(dest_path).exists(),
        "no file should be created on the host at the remote worktree's path string"
    );

    drop(_src_dir);
}

#[tokio::test]
async fn materialize_external_paths_noop_when_no_absolute_paths() {
    // Prompt with no backtick-quoted absolute paths → nothing to copy,
    // prompt returned unchanged.
    let exec = RecordingExec::new();
    let prompt = "You are an implementation engineer.\n\nFollow the spec.";
    let remote_wt = "/home/builder/repo_wt_x";
    let rewritten =
        materialize_external_artifact_paths(prompt, remote_wt, &exec, "m-builder").await;
    assert_eq!(rewritten, prompt);
    assert_eq!(exec.write_count(), 0, "no writes expected for empty prompt");
}

#[tokio::test]
async fn materialize_external_paths_skips_paths_inside_worktree() {
    // An absolute path that already sits inside the worktree (e.g.
    // produced by an earlier materialize step) must be left alone —
    // it's already readable under external_directory: deny.
    let exec = RecordingExec::new();
    let inside_wt = "/home/builder/repo_wt_x/artifacts/_context/already-here.md";
    let prompt = format!("- `{inside_wt}`\n\nbody");
    let rewritten =
        materialize_external_artifact_paths(&prompt, "/home/builder/repo_wt_x", &exec, "m-builder")
            .await;
    assert_eq!(
        rewritten, prompt,
        "paths inside the worktree must NOT be rewritten"
    );
    assert_eq!(exec.write_count(), 0);
}

#[tokio::test]
async fn materialize_external_paths_local_machine_routes_through_exec() {
    // Local-machine regression: same machinery, same fix. The path
    // gets to the right place via exec.write_file (which the local
    // adapter implements as std::fs under the hood).
    let (_src_dir, src_path) = temp_artifact("s-implement.md", "## Files\n");
    let src_str = src_path.to_string_lossy().to_string();

    let local_wt = std::env::temp_dir().join(format!(
        "demeteo_mat_local_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&local_wt).unwrap();

    let exec = RecordingExec::new();
    let prompt = format!("- `{src_str}`\n");
    let rewritten =
        materialize_external_artifact_paths(&prompt, &local_wt.to_string_lossy(), &exec, "local")
            .await;

    assert_eq!(exec.write_count(), 1);
    let (machine_id, dest_path, content) = &exec.recorded_writes()[0];
    assert_eq!(machine_id, "local");
    assert!(dest_path.starts_with(&local_wt.to_string_lossy().to_string()));
    assert_eq!(content, "## Files\n");
    assert!(rewritten.contains(dest_path));

    // Clean up the worktree dest.
    let _ = std::fs::remove_dir_all(&local_wt);
    drop(_src_dir);
}

// ── tempdir re-implementation ───────────────────────────────────────────
//
// The workspace tests use a tiny `tempdir` crate (the standalone
// `tempdir` re-export). It isn't on this crate's dev-dependencies
// for the production build, so we inline a 4-line equivalent here so
// the materialize tests don't pull a new dep.
mod tempdir {
    pub struct TempDir(std::path::PathBuf);
    impl TempDir {
        pub fn from_path(p: std::path::PathBuf) -> Self {
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
