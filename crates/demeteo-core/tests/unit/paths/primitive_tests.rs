// Tests extracted from `crates/demeteo-core/src/paths.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn now_ms_is_monotonic_and_positive() {
    let a = now_ms();
    let b = now_ms();
    assert!(a > 0);
    assert!(b >= a);
}

#[test]
fn new_id_is_16_hex_chars_and_unique_enough() {
    let a = new_id();
    let b = new_id();
    assert_eq!(a.len(), 16);
    assert_eq!(b.len(), 16);
    assert_ne!(a, b);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

/// `MachineId::is_local` accepts two spellings — `"local"` and the empty string
/// a caller that never had a machine to name arrives with — and a predicate
/// that knows only one of them sends the wrong platform's answer to whichever
/// call site skips the resolver.
#[test]
fn both_spellings_of_the_local_machine_are_the_windows_host() {
    assert!(windows_host_target(true, "local"));
    assert!(windows_host_target(true, ""));
    assert!(!windows_host_target(true, "build-box"));
}

/// A remote machine is Linux whatever the desktop is, so a Windows host must
/// still emit the POSIX answer for it — and a Linux host has no Windows target
/// at all.
#[test]
fn a_windows_desktop_driving_a_remote_is_not_a_windows_target() {
    assert!(!windows_host_target(true, "runner-1"));
    assert!(!windows_host_target(false, "local"));
    assert!(!windows_host_target(false, "runner-1"));
}

/// The three producers, on one location: a `PathBuf`, git, and Git Bash.
#[test]
fn the_three_windows_spellings_reduce_to_the_one_the_host_uses() {
    let native = std::path::PathBuf::from(r"C:\Users\runneradmin\demeteo");
    for spelling in [
        r"C:\Users\runneradmin\demeteo",
        "C:/Users/runneradmin/demeteo",
        "/c/Users/runneradmin/demeteo",
    ] {
        assert_eq!(native_path(spelling, true), native, "{spelling}");
    }
    assert_eq!(native_path("/c", true), std::path::PathBuf::from(r"C:\"));
    assert_eq!(native_path("/c/", true), std::path::PathBuf::from(r"C:\"));
}

/// A drive letter is the one component that arrives in both cases — MSYS
/// lowercases it, a `PathBuf` does not — and NTFS folds the rest of the path
/// too.
#[test]
fn windows_paths_compare_case_insensitively() {
    assert!(same_path(
        "/c/Users/Runner/Demeteo",
        r"c:\users\runner\demeteo",
        true
    ));
    assert!(same_path(r"C:\a\b\", "C:/a/b", true));
    assert!(!same_path(r"C:\a\b", r"C:\a\c", true));
    assert!(!same_path(r"C:\a\b", r"D:\a\b", true));
}

/// A remote machine is Linux whatever the desktop is, so `/c/...` there is an
/// ordinary directory: rewriting it would invent a drive, and folding case
/// would merge two directories a case-sensitive filesystem keeps apart.
#[test]
fn a_non_windows_target_keeps_its_paths_verbatim() {
    assert_eq!(
        native_path("/c/Users/runner", false),
        std::path::PathBuf::from("/c/Users/runner")
    );
    assert!(!same_path("/c/Users/runner", r"C:\Users\runner", false));
    assert!(!same_path("/srv/Repo", "/srv/repo", false));
    assert!(same_path("/srv/repo/", "/srv/repo", false));
}

/// The Windows desktop is the only host that ever reads a POSIX path back from
/// somewhere else, and `Path::is_absolute` compiled for Windows calls every one
/// of them relative.
#[test]
fn a_posix_path_is_absolute_for_the_posix_target_that_reported_it() {
    assert!(is_absolute_on("/srv/p/worktrees/one", false));
    assert!(!is_absolute_on("srv/p", false));
    assert!(!is_absolute_on(r"C:\Users\runner", false));
}

/// A drive letter or a UNC root, in either slash — git answers `C:/…` on every
/// platform and a `PathBuf` answers `C:\…`. A bare `C:` is drive-*relative* and
/// names a different directory per drive, so it is not one of them.
#[test]
fn a_windows_path_is_absolute_only_with_a_drive_or_a_unc_root() {
    assert!(is_absolute_on(r"C:\Users\runner", true));
    assert!(is_absolute_on("C:/Users/runner", true));
    assert!(is_absolute_on(r"\\?\C:\Users\runner", true));
    assert!(is_absolute_on(r"\\server\share\repo", true));
    assert!(!is_absolute_on("C:", true));
    assert!(!is_absolute_on(r"\Users\runner", true));
    assert!(!is_absolute_on("Users/runner", true));
}

/// One path manifest holds both: the artifact the desktop wrote and the
/// worktree the step will read it in.
#[test]
fn a_manifest_path_is_recognised_whichever_host_wrote_it() {
    assert!(looks_absolute("/home/builder/wt/artifacts/plan.md"));
    assert!(looks_absolute(r"C:\Users\runner\AppData\artifacts\plan.md"));
    assert!(!looks_absolute("artifacts/plan.md"));
    assert!(!looks_absolute(""));
}

/// The separator belongs to the machine the path is *on*, not to the one
/// composing it — the whole reason this exists rather than `Path::join`.
#[test]
fn a_join_uses_the_owning_hosts_separator() {
    assert_eq!(
        join_on("/home/builder/repo_wt_x", ["artifacts", "_context"], false),
        "/home/builder/repo_wt_x/artifacts/_context"
    );
    assert_eq!(
        join_on(r"C:\w\repo_wt_x", ["artifacts", "_context"], true),
        r"C:\w\repo_wt_x\artifacts\_context"
    );
}

/// A base already carrying a separator must not double it, and a root is all
/// separator — `//artifacts` is a UNC path on Windows and a reserved spelling
/// in POSIX, so neither is the directory that was meant.
#[test]
fn a_join_onto_a_root_or_a_trailing_separator_adds_exactly_one() {
    assert_eq!(join_on("/", ["artifacts"], false), "/artifacts");
    assert_eq!(
        join_on("/home/builder/wt/", ["artifacts"], false),
        "/home/builder/wt/artifacts"
    );
    assert_eq!(join_on(r"C:\", ["artifacts"], true), r"C:\artifacts");
    assert_eq!(
        join_on(r"C:\w\wt\", ["artifacts"], true),
        r"C:\w\wt\artifacts"
    );
}

/// Ids that share a prefix are the norm here — every id starts with a tag and
/// the high digits of a wall clock — so the segment has to read the whole
/// string.
#[test]
fn short_segments_separate_ids_that_share_a_long_prefix() {
    let a = short_path_segment("p1781624953648");
    let b = short_path_segment("p1781624953649");
    assert_ne!(a, b);
    assert_eq!(a.len(), SHORT_SEGMENT_LEN);
    assert_eq!(short_path_segment("p1781624953648"), a);
}
