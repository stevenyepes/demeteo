//! What a spawned tree is confined to.
//!
//! The job itself is a syscall's worth of `cfg(windows)` and only a Windows
//! machine can watch it reap anything. What it *decides* — which limits — is
//! reachable from the host that has no Windows, which is the only place
//! anybody sees it before CI.

use super::*;

#[test]
fn the_job_reaps_its_tree_but_lets_a_process_that_asks_break_away() {
    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const BREAKAWAY_OK: u32 = 0x800;
    const SILENT_BREAKAWAY_OK: u32 = 0x1000;

    assert_eq!(JOB_LIMIT_FLAGS & KILL_ON_JOB_CLOSE, KILL_ON_JOB_CLOSE);
    assert_eq!(JOB_LIMIT_FLAGS & BREAKAWAY_OK, BREAKAWAY_OK);
    assert_eq!(
        JOB_LIMIT_FLAGS & SILENT_BREAKAWAY_OK,
        0,
        "silent breakaway would let every agent child leave the tree unasked — see the constant"
    );
}
