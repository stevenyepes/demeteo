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
