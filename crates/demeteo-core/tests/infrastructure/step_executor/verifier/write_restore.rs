// Tests for the pre-harness write-restore's mechanism choice in
// `src/adapters/step_executor/driver/verifier/mod.rs` (mirrored-tests
// convention). `super` resolves to that module.
//
// The mapping exists so the Windows arm of the choice is reachable from a Linux
// test at all: inside the `match` it is `cfg(windows)`-shaped code that no local
// gate ever compiles, let alone executes.

use super::{write_restore, WriteRestore};

const UNIX_HOST: bool = false;
const WINDOWS_HOST: bool = true;

#[test]
fn a_unix_host_lifts_every_fence_with_chmod() {
    assert_eq!(write_restore(UNIX_HOST, "local"), WriteRestore::Chmod);
    assert_eq!(write_restore(UNIX_HOST, ""), WriteRestore::Chmod);
    assert_eq!(write_restore(UNIX_HOST, "m-builder"), WriteRestore::Chmod);
}

/// The regression this pairing exists for. A Windows-local worktree carries the
/// deny ACE, not a stripped mode bit, so the `chmod` lifts nothing — and the
/// harness commands that run next are the ones that need `target/` writable.
/// Skipping the restore entirely, which is what this did while the Windows fence
/// was still `docs/WINDOWS_PARITY.md`'s Phase 4, now wedges every retried step
/// on a red build no ticket can close.
#[test]
fn a_windows_local_worktree_is_lifted_by_revoking_the_deny_ace() {
    assert_eq!(
        write_restore(WINDOWS_HOST, "local"),
        WriteRestore::RevokeDenyAce
    );
}

/// The empty machine id means the desktop host too. A check that tested one
/// spelling and not the other would send a `chmod` to a Windows shell for every
/// caller that skips `machine_resolver`.
#[test]
fn a_windows_host_reads_the_unnamed_machine_as_its_own() {
    assert_eq!(write_restore(WINDOWS_HOST, ""), WriteRestore::RevokeDenyAce);
}

/// The one the host's OS must not decide alone: a remote machine is always
/// Linux, the `chmod a-w` fence applied there, and `chmod -R u+w` is still its
/// inverse. Revoking an ACE on a machine that has no ACLs restores nothing.
#[test]
fn a_windows_desktop_driving_a_remote_machine_still_chmods() {
    assert_eq!(
        write_restore(WINDOWS_HOST, "m-builder"),
        WriteRestore::Chmod
    );
}
