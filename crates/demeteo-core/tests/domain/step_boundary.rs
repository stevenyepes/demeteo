//! What each capability is told it must not do.
//!
//! Moved verbatim from `tests/infrastructure/step_executor/artifacts/attached.rs`.
//! No doubles: the block is pure over a capability and an already-resolved
//! profile.

use super::*;

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
