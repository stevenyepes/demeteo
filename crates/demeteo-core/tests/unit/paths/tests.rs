// Tests extracted from `crates/demeteo-core/src/paths.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn repo_name_from_path_handles_typical_inputs() {
    assert_eq!(repo_name_from_path("prototype/spectacular"), "spectacular");
    assert_eq!(repo_name_from_path("spectacular"), "spectacular");
    assert_eq!(repo_name_from_path("a/b/c/d"), "d");
    assert_eq!(repo_name_from_path("a/b/"), "b");
}
