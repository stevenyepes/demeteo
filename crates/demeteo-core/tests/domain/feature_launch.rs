// Tests extracted from `crates/demeteo-core/src/domain/feature_launch.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn a_blank_title_is_refused_before_the_description_is_looked_at() {
    assert_eq!(
        refuse_blank_launch_text("  \t", ""),
        Err("Feature title cannot be empty.".to_string())
    );
}

#[test]
fn a_blank_description_is_refused() {
    assert_eq!(
        refuse_blank_launch_text("t", " \n"),
        Err("Feature description cannot be empty.".to_string())
    );
}

#[test]
fn text_in_both_is_accepted() {
    assert_eq!(refuse_blank_launch_text("t", "d"), Ok(()));
}
