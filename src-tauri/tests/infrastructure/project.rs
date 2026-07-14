// Tests extracted from `src-tauri/src/commands/project.rs` (mirrored-tests
// convention). `super` = that module.

use super::parse_effort_param;
use demeteo_core::domain::models::EffortLevel;

#[test]
fn omitted_effort_is_inherit() {
    assert_eq!(parse_effort_param(None).unwrap(), None);
}

#[test]
fn blank_effort_from_a_cleared_select_is_inherit_not_an_error() {
    // The UI clears a select by sending "". That is "inherit", the same
    // thing an omitted field means — never a parse failure.
    assert_eq!(parse_effort_param(Some(String::new())).unwrap(), None);
    assert_eq!(parse_effort_param(Some("  ".to_string())).unwrap(), None);
}

#[test]
fn a_real_level_parses_from_its_canonical_lowercase_spelling() {
    assert_eq!(
        parse_effort_param(Some("xhigh".to_string())).unwrap(),
        Some(EffortLevel::XHigh)
    );
    assert_eq!(
        parse_effort_param(Some("low".to_string())).unwrap(),
        Some(EffortLevel::Low)
    );
}

#[test]
fn an_unknown_level_is_a_validation_error() {
    // Only a frontend bug can produce this; degrading it to "inherit" would
    // hide the bug behind a run that quietly used the wrong effort.
    assert!(parse_effort_param(Some("turbo".to_string())).is_err());
}
