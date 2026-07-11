// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/agent/spawn.rs` (mirrored-tests convention). `super` = that module.

use super::cached_session_needs_respawn;

const WT: &str = "/repos/demeteo_wt_f-1_step-s-spec";

#[test]
fn reuses_live_session_in_matching_worktree() {
    // The normal same-worktree case (retry, parallel subtask): keep it.
    assert!(!cached_session_needs_respawn(true, WT, WT));
}

#[test]
fn respawns_on_worktree_mismatch() {
    // The bug this guards: a live session created in the research
    // worktree, reused by the spec step in a *different* worktree —
    // opencode would write the spec into the research worktree.
    let research_wt = "/repos/demeteo_wt_f-1_step-s-research";
    assert!(cached_session_needs_respawn(true, research_wt, WT));
}

#[test]
fn respawns_when_dead_even_if_worktree_matches() {
    assert!(cached_session_needs_respawn(false, WT, WT));
}

#[test]
fn untracked_cwd_never_forces_respawn_on_worktree_axis() {
    // Stubs/noop report `""` — only liveness governs them.
    assert!(!cached_session_needs_respawn(true, "", WT));
    assert!(cached_session_needs_respawn(false, "", WT));
}
