//! What a step is told it must not do — the prompt-level counterpart to the
//! OS-level fence.
//!
//! It sits beside [`permission`](crate::domain::permission), whose
//! `resolve_profile` produces its second argument. It only ever **reads** the
//! compiled [`PermissionProfile`](crate::domain::permission::PermissionProfile);
//! it never constructs or widens one, and there is no `ask` branch to add — the
//! profile is complete and uses only allow/deny by invariant (AGENTS.md §2).
//! This is prompt text. It is not the fence.

use crate::domain::permission::{PermissionProfile, StepCapability};

/// Prepend a prohibitive **Operating Boundary** block describing what the
/// step's capability forbids — the prompt-level counterpart to the OS-level
/// fence and the agent's tool policy. Where
/// [`inject_artifact_contract`](crate::domain::artifact_contract::inject_artifact_contract)
/// tells the agent *what to produce*, this tells it *what it must not do*,
/// in imperative MUST/MUST NOT language that survives a redirected step
/// trying to "just fix it".
///
/// The block is keyed on the [`StepCapability`] (role) and refined by the
/// resolved [`PermissionProfile`] so the shell/network lines match any
/// per-step `allow_shell` / `allow_network` widening. `Implement` steps get
/// no block (full access — nothing to forbid).
///
/// Returned at the *front* of the prompt: a boundary the model reads first
/// outranks instructions buried in a long template that might tempt it to
/// implement.
pub(crate) fn inject_operating_boundary(
    prompt: &str,
    capability: StepCapability,
    profile: &PermissionProfile,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    let (mode, rules): (&str, Vec<String>) = match capability {
        StepCapability::Implement => (
            "IMPLEMENT",
            vec![
                "You have full read/write access to the worktree, including source, tests, \
                 configuration, documentation, and any other tracked or untracked file."
                    .to_string(),
                "There is no separate report folder for this step. Write the deliverable \
                 directly at the path the task describes — e.g. the real repo path the gate \
                 approved (`docs/<area>/<topic>.md` for a docs-update workflow, the function \
                 under test for a refactor, the failing line for a bugfix)."
                    .to_string(),
                "Any change you write to a repo path (NOT under the project's report \
                 subdir) is committed to the feature branch automatically. Any change you \
                 write under the report subdir (the per-step change-summary folder the \
                 orchestrator surfaces in the UI, named after your step id) stays in the \
                 worktree as an untracked file unless the project has opted into \
                 `commit_artifacts`."
                    .to_string(),
            ],
        ),
        StepCapability::ReadOnly => (
            "REVIEW-ONLY",
            vec![
                "You MUST NOT create, edit, move, or delete any file.".to_string(),
                "You MUST NOT modify source code, configuration, or artifacts.".to_string(),
                "Your job is to inspect and report — produce your assessment as \
                 text in your response."
                    .to_string(),
            ],
        ),
        StepCapability::Artifacts => (
            "ANALYSIS",
            vec![
                "You may ONLY write files under the `artifacts/` directory.".to_string(),
                "You MUST NOT modify source code, tests, configuration, or any \
                 file outside `artifacts/`."
                    .to_string(),
                "If the task appears to call for code changes, do NOT make them — \
                 that is a later implementation step's job. Capture your findings, \
                 spec, or plan in your artifact instead."
                    .to_string(),
            ],
        ),
        StepCapability::Verify => (
            "VALIDATION",
            vec![
                "You may run build/test/lint/audit commands and read any file.".to_string(),
                "You may ONLY write files under the `artifacts/` directory (your report)."
                    .to_string(),
                "You MUST NOT fix or modify source code. If you find problems, \
                 document them precisely in your artifact so an implementation \
                 step can address them."
                    .to_string(),
            ],
        ),
    };

    lines.push(format!("## Operating Boundary — {} mode", mode));
    lines.push(String::new());
    lines.push(
        "These constraints are enforced by the orchestrator (the filesystem is \
         fenced and out-of-scope writes are reverted and fail the step). Staying \
         inside them is part of completing the task:"
            .to_string(),
    );
    lines.push(String::new());
    for r in rules {
        lines.push(format!("- {}", r));
    }

    // Shell / network lines reflect the *resolved* profile so per-step
    // widenings (allow_shell / allow_network) don't contradict the block.
    if !profile.execute.is_allow() {
        lines.push("- You MUST NOT run shell commands.".to_string());
    }
    if profile.network.is_allow() {
        lines.push(
            "- You MAY use web search/fetch to consult up-to-date documentation.".to_string(),
        );
    } else {
        lines.push("- You MUST NOT access the network.".to_string());
    }

    lines.push(String::new());
    lines.push("---".to_string());
    lines.push(String::new());

    format!("{}{}", lines.join("\n"), prompt)
}

#[cfg(test)]
#[path = "../../tests/domain/step_boundary.rs"]
mod tests;
