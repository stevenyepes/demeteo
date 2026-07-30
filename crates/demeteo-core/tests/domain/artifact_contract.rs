//! The `## Expected Artifacts` block, over declarations alone.
//!
//! Moved verbatim from `tests/infrastructure/step_executor/artifacts/attached.rs`.

use super::*;
use crate::domain::artifact::{ArtifactCapture, ArtifactDecl};

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
