// Tests for the write-restore predicate in
// `src/adapters/step_executor/driver/verifier/mod.rs` (mirrored-tests
// convention). `super` resolves to that module.
//
// The predicate exists so the Windows arm of a `chmod` decision is reachable
// from a Linux test run at all: inside the `if` it was `cfg(windows)`-shaped
// code that no local gate ever compiles, let alone executes.

use super::restores_write_access;

const UNIX_HOST: bool = false;
const WINDOWS_HOST: bool = true;

#[test]
fn a_unix_host_restores_write_access_everywhere() {
    assert!(restores_write_access(UNIX_HOST, "local"));
    assert!(restores_write_access(UNIX_HOST, ""));
    assert!(restores_write_access(UNIX_HOST, "m-builder"));
}

/// Nothing has been made read-only on a Windows host — the ACL fence is Phase 4
/// — so there is nothing to restore, and a `chmod` there would be a lie about a
/// fence that was never applied.
#[test]
fn a_windows_host_skips_the_restore_on_its_own_machine() {
    assert!(!restores_write_access(WINDOWS_HOST, "local"));
}

/// The empty machine id means the desktop host too. A check that tested one
/// spelling and not the other would send a `chmod` to a Windows shell for every
/// caller that skips `machine_resolver`.
#[test]
fn a_windows_host_skips_the_restore_for_the_unnamed_machine() {
    assert!(!restores_write_access(WINDOWS_HOST, ""));
}

/// The one the host's OS must not decide alone: a remote machine is always
/// Linux, the Unix fence applied there, and the restore is still its inverse.
#[test]
fn a_windows_host_still_restores_on_a_remote_machine() {
    assert!(restores_write_access(WINDOWS_HOST, "m-builder"));
}
