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
