// Tests for `steps/finalize/mod.rs` (mirrored-tests convention).
// `super` = that module.

use super::*;

/// The load-bearing safety property of the whole design: the finalize agent
/// runs without a shell, so it *cannot* invoke `gh`/`glab`/`curl` to open the
/// PR itself — Demeteo does that, through the provider's HTTP API. This is
/// enforcement, not instruction, and this test pins it: widening the finalize
/// capability (or letting a workflow's `allow_shell` reach it) breaks here.
#[test]
fn the_finalize_agent_has_no_shell() {
    use crate::adapters::agent::claude_code::disallowed_tools_for;
    use crate::domain::permission::resolve_profile;

    let profile = resolve_profile(ExecutionDriver::finalize_capability(), false, false);
    assert!(
        !profile.execute.is_allow(),
        "the finalize agent must never be allowed to execute commands"
    );
    assert!(
        !profile.network.is_allow(),
        "the finalize agent has no reason to reach the network"
    );

    let denied = disallowed_tools_for(&profile);
    assert!(
        denied.contains(&"Bash"),
        "Bash must be denied to the finalize agent — without that, nothing stops it \
         from running `gh pr create`. Denied set was: {denied:?}"
    );
}
