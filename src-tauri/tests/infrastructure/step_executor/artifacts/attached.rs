use super::*;
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
            created_at: 0,
            updated_at: 0,
        },
        StepExecution {
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
fn boundary_implement_is_a_noop() {
    let prompt = "do the work";
    let out = inject_operating_boundary(
        prompt,
        StepCapability::Implement,
        &PermissionProfile::all_allow(),
    );
    assert_eq!(out, prompt, "Implement steps get no boundary block");
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
