use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
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
                            if let Some(ref path) = prev_step.artifact_path {
                                if let Ok(art_content) = std::fs::read_to_string(path) {
                                    replacement = art_content;
                                } else {
                                    replacement =
                                        format!("(Error reading artifact at {})", path);
                                }
                            } else {
                                replacement =
                                    "(No artifact path found for previous step)".to_string();
                            }
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
                            if let Some(ref path) = s.artifact_path {
                                if let Ok(art_content) = std::fs::read_to_string(path) {
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
                error_message: None,
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
                error_message: None,
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
}
