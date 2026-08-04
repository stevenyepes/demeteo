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
