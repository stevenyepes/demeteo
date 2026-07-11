// Tests extracted from `crates/demeteo-core/src/shared/ids.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn ids_are_unique_under_burst() {
    let mut ids = std::collections::HashSet::new();
    for _ in 0..1000 {
        ids.insert(new_id());
    }
    assert_eq!(ids.len(), 1000, "duplicate IDs in a burst");
}

#[test]
fn ids_sort_lexicographically_by_time() {
    let a = new_id();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let b = new_id();
    // Both ids are hex; the millisecond prefix dominates.
    assert!(a < b, "expected {} < {}", a, b);
}

#[test]
fn ids_have_three_components() {
    let id = new_id();
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 3, "expected 3 components in {}", id);
}
