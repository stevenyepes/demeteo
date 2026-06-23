use super::*;
use crate::domain::ids::FeatureId;
use crate::domain::ids::StepExecutionId;
use crate::ports::artifact_store::ArtifactStore;
use std::sync::Arc;

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

    let template = "Read the research: [attached — s-research] and the spec: [attached — s-spec]";
    let resolved = resolve_attached_artifacts(template, &step_execs, 1, &*store);
    assert_eq!(
        resolved,
        "=== ATTACHED CONTEXT: s-research ===\nThis is the research content.\n================================\n\n=== ATTACHED CONTEXT: s-spec ===\nThis is the spec content.\n================================\n\nRead the research: [See attached s-research at the beginning of the prompt] and the spec: [See attached s-spec at the beginning of the prompt]"
    );

    let template_prev = "Previous content: [attached — previous step artifact]";
    let resolved_prev = resolve_attached_artifacts(template_prev, &step_execs, 1, &*store);
    assert_eq!(
        resolved_prev,
        "=== ATTACHED CONTEXT: s-research ===\nThis is the research content.\n================================\n\nPrevious content: [See attached s-research at the beginning of the prompt]"
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
        },
        ArtifactDecl {
            name: "diff".into(),
            capture: ArtifactCapture::Diff {
                base: DiffBase::WorktreeBase,
                path_filter: None,
            },
            mode: crate::domain::artifact::ArtifactMode::Full,
        },
        ArtifactDecl {
            name: "wt".into(),
            capture: ArtifactCapture::Worktree {
                path: Some("src/".into()),
            },
            mode: crate::domain::artifact::ArtifactMode::None,
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
    let resolved = resolve_attached_artifacts(template, &step_execs, 1, &*store);
    assert_eq!(
        resolved,
        "=== ATTACHED CONTEXT: s-research ===\nResearch content from paths.\n================================\n\nPrevious: [See attached s-research at the beginning of the prompt]"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}
