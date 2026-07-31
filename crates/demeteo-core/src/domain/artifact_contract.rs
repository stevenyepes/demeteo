//! The `## Expected Artifacts` block: what the orchestrator expects this step
//! to produce, stated to the agent in the contract's own vocabulary rather than
//! repeated in prose by every prompt author.

use crate::domain::artifact::{ArtifactCapture, ArtifactDecl};

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
                "- Every file you write will be captured automatically via git".to_string()
            }
            ArtifactCapture::ChangedFiles { path_filter, .. } => {
                if let Some(filter) = path_filter {
                    format!(
                        "- All files matching `{}` will be captured automatically via git",
                        filter
                    )
                } else {
                    "- All changed files will be captured automatically via git".to_string()
                }
            }
            ArtifactCapture::Diff { .. } => {
                "- A diff will be computed at the end of the step".to_string()
            }
            ArtifactCapture::Worktree { path: Some(p) } => {
                format!("- Worktree pointer for `{}`", p)
            }
            ArtifactCapture::Worktree { path: None } => "- Worktree root pointer".to_string(),
        };
        lines.push(hint);
    }

    lines.push(String::new());
    lines.push(
        "Your file changes are automatically detected via git — no special naming required."
            .to_string(),
    );

    let mut result = prompt.to_string();
    result.push_str(&lines.join("\n"));
    result
}

#[cfg(test)]
#[path = "../../tests/domain/artifact_contract.rs"]
mod tests;
