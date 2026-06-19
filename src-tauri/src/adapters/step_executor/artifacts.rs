use std::sync::Arc;

use crate::domain::artifact::{Artifact, ArtifactCapture, ArtifactDecl, ArtifactSource};
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::GateRepository;

/// Returns `(decision, feedback)` from the most recently *decided* gate step
/// for a feature.  Used to inject `{{gate_decision}}` and `{{gate_feedback}}`
/// into the next agent step's rendered prompt.
///
/// Best-effort: returns `("", "")` when no gate has been decided yet (the
/// common case for the first agent step in any workflow).
pub(crate) fn get_latest_gate_decision(
    gates: &dyn GateRepository,
    feature_id: &str,
) -> (String, String) {
    let f_id = FeatureId::from(feature_id.to_string());
    match gates.latest_decided_for_feature(&f_id) {
        Ok(Some(decided)) => (
            decided.decision.unwrap_or_default(),
            decided.feedback.unwrap_or_default(),
        ),
        _ => (String::new(), String::new()),
    }
}

/// Resolve `[attached — <step_id>]` and `[attached — previous step artifact]`
/// placeholders inside a prompt template by reading the corresponding artifact
/// files from disk.
pub(crate) fn resolve_attached_artifacts(
    prompt: &str,
    step_execs: &[StepExecution],
    step_index: usize,
) -> String {
    let mut resolved_prompt = prompt.to_string();
    let mut search_start = 0;

    while let Some(start_idx) = resolved_prompt[search_start..].find("[attached") {
        let absolute_start = search_start + start_idx;
        if let Some(end_offset) = resolved_prompt[absolute_start..].find(']') {
            let absolute_end = absolute_start + end_offset;
            let full_placeholder = &resolved_prompt[absolute_start..=absolute_end];

            let inside = &full_placeholder[1..full_placeholder.len() - 1];

            let parts: Vec<&str> = if inside.contains('\u{2014}') {
                inside.split('\u{2014}').collect()
            } else if inside.contains('\u{2013}') {
                inside.split('\u{2013}').collect()
            } else {
                inside.split('-').collect()
            };

            if parts.len() >= 2 {
                let content = parts[1].trim();
                let mut replacement = String::new();

                if content == "previous step artifact" {
                    if step_index > 0 {
                        if let Some(prev_step) = step_execs.get(step_index - 1) {
                            let paths: Vec<&String> = if !prev_step.artifact_paths.is_empty() {
                                prev_step.artifact_paths.iter().collect()
                            } else {
                                prev_step.artifact_path.as_ref().into_iter().collect()
                            };
                            let mut parts = Vec::new();
                            for p in &paths {
                                match std::fs::read_to_string(p) {
                                    Ok(c) => parts.push(c),
                                    Err(_) => parts.push(format!("(Error reading artifact at {})", p)),
                                }
                            }
                            replacement = if parts.len() == 1 {
                                parts.into_iter().next().unwrap_or_default()
                            } else {
                                parts.join("\n\n---\n\n")
                            };
                        }
                    } else {
                        replacement = "(No previous step exists)".to_string();
                    }
                } else {
                    let mut found = false;
                    let mut matched_contents = Vec::new();

                    for s in step_execs {
                        let sid = s.step_id.0.to_lowercase();
                        let content_lower = content.to_lowercase();

                        if content_lower.contains(&sid) || sid.contains(&content_lower) {
                            let paths: Vec<&String> = if !s.artifact_paths.is_empty() {
                                s.artifact_paths.iter().collect()
                            } else {
                                s.artifact_path.as_ref().into_iter().collect()
                            };
                            for p in &paths {
                                if let Ok(art_content) = std::fs::read_to_string(p) {
                                    matched_contents.push(format!(
                                        "### Artifact from Step {}\n\n{}",
                                        s.step_id.0, art_content
                                    ));
                                    found = true;
                                }
                            }
                        }
                    }

                    if found {
                        replacement = matched_contents.join("\n\n");
                    } else {
                        replacement = format!(
                            "(Artifact '{}' not found or not yet generated)",
                            content
                        );
                    }
                }

                resolved_prompt = resolved_prompt.replace(full_placeholder, &replacement);
                search_start = 0;
                continue;
            }
        }
        search_start += start_idx + 1;
    }

    resolved_prompt
}

/// Append a synthetic `## Expected Artifacts (orchestrator contract)` block
/// to `prompt` when `declarations` is non-empty. The agent sees exactly
/// which named artifacts the orchestrator expects and where to write
/// them, without the prompt author having to repeat the contract in
/// natural-language prose.
///
/// Returns the original `prompt` unchanged when `declarations` is
/// `None` or empty (legacy backstop).
pub(crate) fn inject_artifact_contract(
    prompt: &str,
    declarations: Option<&[ArtifactDecl]>,
) -> String {
    let decls = match declarations {
        Some(d) if !d.is_empty() => d,
        _ => return prompt.to_string(),
    };

    let mut lines = vec![
        String::new(),
        "## Expected Artifacts (orchestrator contract)".to_string(),
        String::new(),
        "Capture your work in the following files so downstream".to_string(),
        "steps and the reviewer can see what you produced:".to_string(),
        String::new(),
    ];

    for d in decls {
        let hint = match &d.capture {
            ArtifactCapture::ByName { name } => {
                format!("- Produce an artifact named `{}`", name)
            }
            ArtifactCapture::LastWriteTo { path } => {
                format!("- Write `{}` → artifact `{}`", path, d.name)
            }
            ArtifactCapture::AllWrites => {
                "- Every file you write will be captured".to_string()
            }
            ArtifactCapture::Diff { .. } => {
                "- A diff will be computed at the end of the step".to_string()
            }
            ArtifactCapture::Worktree { path: Some(p) } => {
                format!("- Worktree pointer for `{}`", p)
            }
            ArtifactCapture::Worktree { path: None } => {
                "- Worktree root pointer".to_string()
            }
        };
        lines.push(hint);
    }

    lines.push(String::new());
    lines.push(
        "Do **not** change the path or name — the orchestrator depends on them."
            .to_string(),
    );

    let mut result = prompt.to_string();
    result.push_str(&lines.join("\n"));
    result
}

/// Resolve `declarations` against the `ArtifactProduced` events emitted
/// by the agent during a step turn. Writes matching artifacts through
/// the store and returns the list of references (paths for the FS
/// adapter) to persist in `StepExecution.artifact_paths`.
///
/// Artifacts that cannot be matched are silently skipped with a
/// `tracing::warn!` — the step executor will still mark the step as
/// completed successfully; missing artifacts are a prompt-engineering
/// concern, not a runtime failure.
pub(crate) fn resolve_declared_artifacts(
    declarations: &[ArtifactDecl],
    produced: &[Artifact],
    store: &Arc<dyn ArtifactStore>,
    feature_id: &str,
    step_id: &str,
) -> Vec<String> {
    let mut refs = Vec::new();

    for decl in declarations {
        let matched: Option<&Artifact> = match &decl.capture {
            ArtifactCapture::ByName { name } => produced.iter().find(|a| a.name == *name),
            ArtifactCapture::LastWriteTo { path } => produced
                .iter()
                .filter(|a| matches!(&a.source, ArtifactSource::ToolWrite { path: p } if p == path))
                .last(),
            ArtifactCapture::AllWrites => {
                // Collect all tool-write artifacts. We still produce the
                // named artifacts below; the `AllWrites` catch-all emits
                // one artifact per unique path.
                continue; // handled separately below
            }
            ArtifactCapture::Diff { .. } => {
                // Diff artifacts are derived at materialisation time by
                // `GitOpsHelper`. No agent event matches them. The
                // orchestrator should synthesise them at TurnComplete
                // when `GitOpsHelper` methods are available (next step).
                eprintln!(
                    "[artifacts] step={} decl={}: Diff declaration skipped — GitOpsHelper not yet wired",
                    step_id, decl.name,
                );
                continue;
            }
            ArtifactCapture::Worktree { .. } => {
                // Worktree-ref artifacts are synthesised by the executor
                // from branch/machine state. No agent event matches them.
                eprintln!(
                    "[artifacts] step={} decl={}: Worktree declaration skipped — GitOpsHelper not yet wired",
                    step_id, decl.name,
                );
                continue;
            }
        };

        if let Some(artifact) = matched {
            match store.put(feature_id, step_id, artifact) {
                Ok(reference) => refs.push(reference),
                Err(e) => {
                eprintln!(
                    "[artifacts] step={} decl={}: Failed to store artifact: {}",
                    step_id, decl.name, e,
                );
                }
            }
        } else {
            eprintln!(
                "[artifacts] step={} decl={}: No matching ArtifactProduced event",
                step_id, decl.name,
            );
        }
    }

    // Handle `AllWrites` catch-all: collect every unique ToolWrite path.
    let has_all_writes = declarations
        .iter()
        .any(|d| matches!(d.capture, ArtifactCapture::AllWrites));
    if has_all_writes {
        let mut seen_paths = std::collections::HashSet::new();
        for artifact in produced {
            if let ArtifactSource::ToolWrite { path } = &artifact.source {
                if seen_paths.insert(path.clone()) {
                    match store.put(feature_id, step_id, artifact) {
                        Ok(reference) => refs.push(reference),
                        Err(e) => {
                        eprintln!(
                            "[artifacts] step={} path={}: Failed to store AllWrites artifact: {}",
                            step_id, path, e,
                        );
                        }
                    }
                }
            }
        }
    }

    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::StepExecutionId;
    use crate::domain::ids::FeatureId;

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
        let resolved = resolve_attached_artifacts(template, &step_execs, 1);
        assert_eq!(
            resolved,
            "Read the research: ### Artifact from Step s-research\n\nThis is the research content. and the spec: ### Artifact from Step s-spec\n\nThis is the spec content."
        );

        let template_prev = "Previous content: [attached — previous step artifact]";
        let resolved_prev = resolve_attached_artifacts(template_prev, &step_execs, 1);
        assert_eq!(resolved_prev, "Previous content: This is the research content.");

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
        let decls = vec![
            ArtifactDecl::full_path("spec", "docs/spec.md"),
        ];
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
                capture: ArtifactCapture::Diff { base: DiffBase::WorktreeBase, path_filter: None },
                mode: crate::domain::artifact::ArtifactMode::Full,
            },
            ArtifactDecl {
                name: "wt".into(),
                capture: ArtifactCapture::Worktree { path: Some("src/".into()) },
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
    fn test_resolve_declared_artifacts_by_name() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_name_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> =
            Arc::new(crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()));

        let declarations = vec![
            ArtifactDecl::full_path("spec", "docs/spec.md"),
        ];

        let produced = vec![
            Artifact::tool_write("spec", "docs/spec.md", "# My Spec\n"),
        ];

        let refs = resolve_declared_artifacts(
            &declarations,
            &produced,
            &store,
            "f-test",
            "s-impl",
        );

        assert_eq!(refs.len(), 1);
        assert!(refs[0].contains("artifacts/f-test/s-impl/spec"));
        // Verify content was stored
        let content = store.get(&refs[0]).unwrap();
        assert_eq!(content, "# My Spec\n");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_resolve_declared_artifacts_last_write() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_last_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> =
            Arc::new(crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()));

        let declarations = vec![ArtifactDecl {
            name: "final-spec".into(),
                capture: ArtifactCapture::LastWriteTo { path: "docs/spec.md".into() },
            mode: crate::domain::artifact::ArtifactMode::Full,
        }];

        let produced = vec![
            Artifact::tool_write("draft", "docs/spec.md", "# Draft\n"),
            Artifact::tool_write("final", "docs/spec.md", "# Final\n"),
        ];

        let refs = resolve_declared_artifacts(
            &declarations,
            &produced,
            &store,
            "f-test",
            "s-impl",
        );

        assert_eq!(refs.len(), 1);
        let content = store.get(&refs[0]).unwrap();
        assert_eq!(content, "# Final\n");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_resolve_declared_artifacts_all_writes() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_all_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> =
            Arc::new(crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()));

        let declarations = vec![ArtifactDecl {
            name: "all-files".into(),
            capture: ArtifactCapture::AllWrites,
            mode: crate::domain::artifact::ArtifactMode::Full,
        }];

        let produced = vec![
            Artifact::tool_write("f1", "src/lib.rs", "// lib\n"),
            Artifact::tool_write("f2", "src/main.rs", "// main\n"),
            // duplicate path should be deduplicated
            Artifact::tool_write("f1-v2", "src/lib.rs", "// lib v2\n"),
        ];

        let refs = resolve_declared_artifacts(
            &declarations,
            &produced,
            &store,
            "f-test",
            "s-impl",
        );

        // Two unique paths: src/lib.rs (last write wins for content, but ref deduped)
        assert_eq!(refs.len(), 2);

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_resolve_declared_artifacts_skips_diff_and_worktree() {
        use crate::domain::artifact::DiffBase;
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_skip_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> =
            Arc::new(crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()));

        let declarations = vec![
            ArtifactDecl {
                name: "code-diff".into(),
                capture: ArtifactCapture::Diff { base: DiffBase::WorktreeBase, path_filter: None },
                mode: crate::domain::artifact::ArtifactMode::Full,
            },
            ArtifactDecl {
                name: "wt-ref".into(),
                capture: ArtifactCapture::Worktree { path: None },
                mode: crate::domain::artifact::ArtifactMode::None,
            },
        ];

        // Produced has no matching artifact — diff/worktree are derived
        let refs = resolve_declared_artifacts(
            &declarations,
            &[],
            &store,
            "f-test",
            "s-impl",
        );

        assert!(refs.is_empty());

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

        let artifact_file = temp_dir.join("s-research.md");
        std::fs::write(&artifact_file, "Research content from paths.").unwrap();
        let artifact_str = artifact_file.to_string_lossy().to_string();

        let step_execs = vec![
            StepExecution {
                id: StepExecutionId::from("se-1"),
                feature_id: FeatureId::from("f-1"),
                step_id: crate::domain::ids::StepId::from("s-research"),
                step_index: 0,
                step_kind: "agent".to_string(),
                status: "completed".to_string(),
                cost_usd: Some(0.0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: vec![artifact_str],
                error_message: None,
                iteration_count: 0,
                created_at: 0,
                updated_at: 0,
            },
        ];

        let template = "Previous: [attached — previous step artifact]";
        let resolved = resolve_attached_artifacts(template, &step_execs, 1);
        assert_eq!(resolved, "Previous: Research content from paths.");

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
