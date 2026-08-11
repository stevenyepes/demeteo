// Tests extracted from `src/adapters/worktree/git_ops/scope.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::*;
use crate::domain::artifact::{ArtifactCapture, ArtifactDecl, ArtifactMode};
use crate::domain::models::StepConfig;
use crate::domain::permission::StepCapability;

fn last_write_to(name: &str, path: &str) -> ArtifactDecl {
    ArtifactDecl {
        name: name.into(),
        capture: ArtifactCapture::LastWriteTo { path: path.into() },
        mode: ArtifactMode::Full,
        inline: false,
    }
}

fn all_writes(name: &str) -> ArtifactDecl {
    ArtifactDecl {
        name: name.into(),
        capture: ArtifactCapture::AllWrites,
        mode: ArtifactMode::Full,
        inline: false,
    }
}

// ── writable_paths_for_step (what the fence and the guard both call) ─

fn sequence_step(artifacts: Option<Vec<ArtifactDecl>>) -> StepConfig {
    StepConfig {
        id: "s-implement".into(),
        kind: "sequence".into(),
        artifacts,
        ..StepConfig::default()
    }
}

#[test]
fn sequence_step_is_fully_writable_without_an_artifact_declaration() {
    // The regression: `s-implement` dropped its `all_writes` declaration,
    // and a capability-blind derivation read that as "extras only" — so
    // the guard reverted every source file the tickets wrote.
    let paths = writable_paths_for_step(
        &sequence_step(None),
        &["src-tauri/target".to_string(), "node_modules".to_string()],
    );
    assert_eq!(paths, vec![PathBuf::from(ALL_WRITES)]);
}

#[test]
fn sequence_step_is_fully_writable_with_a_last_write_to_declaration() {
    // A declaration names a deliverable; it must not narrow an implement
    // step to that one path.
    let decls = vec![last_write_to("summary", "artifacts/implement-summary.md")];
    let paths = writable_paths_for_step(&sequence_step(Some(decls)), &no_extras());
    assert_eq!(paths, vec![PathBuf::from(ALL_WRITES)]);
}

#[test]
fn artifacts_step_stays_fenced_to_its_declared_paths() {
    let step = StepConfig {
        id: "s-research".into(),
        kind: "agent".into(),
        capability: Some(StepCapability::Artifacts),
        artifacts: Some(vec![last_write_to(
            "report",
            "artifacts/research-report.md",
        )]),
        ..StepConfig::default()
    };
    assert_eq!(
        writable_paths_for_step(&step, &no_extras()),
        vec![
            PathBuf::from(ARTIFACTS_DIR),
            PathBuf::from("artifacts/research-report.md"),
        ]
    );
}

#[test]
fn an_unconstrained_capture_still_infers_implement_for_an_agent_step() {
    // Back-compat: a pre-capability workflow declaring `all_writes` and no
    // capability keeps the whole worktree, via `effective_capability`.
    let step = StepConfig {
        id: "s-legacy".into(),
        kind: "agent".into(),
        artifacts: Some(vec![all_writes("implemented-files")]),
        ..StepConfig::default()
    };
    assert_eq!(
        writable_paths_for_step(&step, &no_extras()),
        vec![PathBuf::from(ALL_WRITES)]
    );
}

#[test]
fn an_agent_step_declaring_nothing_falls_back_to_artifacts_only() {
    let step = StepConfig {
        id: "s-plain".into(),
        kind: "agent".into(),
        ..StepConfig::default()
    };
    assert_eq!(
        writable_paths_for_step(&step, &no_extras()),
        vec![PathBuf::from(ARTIFACTS_DIR)]
    );
}

// ── derive_writable_paths_for_scope (capability-authoritative) ───────

fn no_extras() -> Vec<String> {
    Vec::new()
}

#[test]
fn scope_all_returns_all_writes_sentinel() {
    let paths = derive_writable_paths_for_scope(WriteScope::All, None, &no_extras());
    assert_eq!(paths, vec![PathBuf::from(ALL_WRITES)]);
}

#[test]
fn scope_all_ignores_extras_because_worktree_is_already_writable() {
    // Implement capability already opens the entire worktree; extras
    // are redundant but should never introduce the NONE sentinel or
    // shadow it.
    let extras = vec!["target/".to_string()];
    let paths = derive_writable_paths_for_scope(WriteScope::All, None, &extras);
    assert_eq!(paths, vec![PathBuf::from(ALL_WRITES)]);
}

#[test]
fn scope_none_returns_none_sentinel_without_extras() {
    let paths = derive_writable_paths_for_scope(WriteScope::None, None, &no_extras());
    assert_eq!(paths, vec![PathBuf::from(NONE_WRITABLE)]);
}

#[test]
fn scope_none_with_extras_widens_past_deny_all() {
    // ReadOnly + extras: the user opted the step into specific tool
    // side-effects (e.g. .cache/coverage). The NONE sentinel is
    // suppressed and the extras become the writable set directly.
    let extras = vec![".cache/coverage".to_string(), "scratch/".to_string()];
    let paths = derive_writable_paths_for_scope(WriteScope::None, None, &extras);
    assert_eq!(
        paths,
        vec![PathBuf::from(".cache/coverage"), PathBuf::from("scratch"),]
    );
    assert!(!paths.iter().any(|p| p == &PathBuf::from(NONE_WRITABLE)));
}

#[test]
fn scope_artifacts_defaults_to_artifacts_dir_when_no_decls() {
    let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &no_extras());
    assert_eq!(paths, vec![PathBuf::from(ARTIFACTS_DIR)]);
}

#[test]
fn scope_artifacts_includes_explicit_last_write_to_paths() {
    let decls = vec![last_write_to("spec", "artifacts/spec.md")];
    let paths =
        derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, Some(&decls), &no_extras());
    assert_eq!(
        paths,
        vec![
            PathBuf::from(ARTIFACTS_DIR),
            PathBuf::from("artifacts/spec.md")
        ]
    );
}

#[test]
fn scope_artifacts_does_not_widen_for_unconstrained_capture() {
    // Even if an artifact-scoped step declares AllWrites, the
    // capability is authoritative: it stays fenced to artifacts/.
    let decls = vec![all_writes("everything")];
    let paths =
        derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, Some(&decls), &no_extras());
    assert_eq!(paths, vec![PathBuf::from(ARTIFACTS_DIR)]);
    assert!(!paths.contains(&PathBuf::from(ALL_WRITES)));
}

#[test]
fn scope_artifacts_appends_extras_after_artifacts_dir() {
    // The canonical use case: a Verify step running `cargo test` on
    // a Rust project. The chmod fence must leave `target/` writable
    // while keeping source read-only.
    let extras = vec!["target/".to_string()];
    let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
    assert_eq!(
        paths,
        vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("target")]
    );
}

#[test]
fn scope_artifacts_dedups_extras_that_overlap_artifacts_dir() {
    // If the user lists `artifacts/` again it must not be appended.
    let extras = vec!["artifacts/".to_string(), "artifacts/extra.md".to_string()];
    let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
    assert_eq!(
        paths,
        vec![
            PathBuf::from(ARTIFACTS_DIR),
            PathBuf::from("artifacts/extra.md"),
        ]
    );
}

// ── extras normalisation (security boundary) ─────────────────────────

#[test]
fn extras_normalisation_strips_trailing_slashes() {
    let paths = derive_writable_paths_for_scope(
        WriteScope::ArtifactsOnly,
        None,
        &["target".to_string(), "node_modules".to_string()],
    );
    assert_eq!(
        paths,
        vec![
            PathBuf::from(ARTIFACTS_DIR),
            PathBuf::from("target"),
            PathBuf::from("node_modules"),
        ]
    );
}

#[test]
fn extras_normalisation_rejects_absolute_paths() {
    // Absolute paths would escape the worktree root. The orchestrator
    // runs on Unix hosts today, where `Path::is_absolute` only
    // recognises paths with a leading `/`; a Windows drive prefix
    // would be treated as a relative path by the shell anyway.
    let extras = vec!["/etc/passwd".to_string(), "/var/log/syslog".to_string()];
    let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
    assert_eq!(paths, vec![PathBuf::from(ARTIFACTS_DIR)]);
}

#[test]
fn extras_normalisation_rejects_parent_dir_escape() {
    // `../foo` would land outside the worktree.
    let extras = vec![
        "../escape".to_string(),
        "ok/../../escape".to_string(),
        "safe".to_string(),
    ];
    let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
    assert_eq!(
        paths,
        vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("safe")]
    );
}

#[test]
fn extras_normalisation_dedups_repeated_entries() {
    let extras = vec![
        "target".to_string(),
        "target/".to_string(),
        "./target".to_string(),
    ];
    let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
    assert_eq!(
        paths,
        vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("target")]
    );
}

#[test]
fn extras_normalisation_skips_empty_entries() {
    let extras = vec!["".to_string(), "   ".to_string(), "target".to_string()];
    let paths = derive_writable_paths_for_scope(WriteScope::ArtifactsOnly, None, &extras);
    assert_eq!(
        paths,
        vec![PathBuf::from(ARTIFACTS_DIR), PathBuf::from("target")]
    );
}
