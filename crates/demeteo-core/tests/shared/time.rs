// Tests extracted from `crates/demeteo-core/src/shared/time.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn now_is_monotonic_within_a_test() {
    let a = now_ms();
    let b = now_ms();
    assert!(b >= a);
}

#[test]
fn seconds_less_than_milliseconds() {
    let ms = now_ms();
    let s = now_secs();
    // ms should be roughly 1000x s, give or take a few ms.
    assert!(ms / 1000 <= s + 1);
    assert!(s * 1000 <= ms + 1000);
}
