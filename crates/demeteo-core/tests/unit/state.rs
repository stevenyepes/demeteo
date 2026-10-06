//! `AppVersion` is shared, not copied: `build_core_context` hands the MCP
//! listener its clone of `AppContext` before `lib.rs` learns the version.

use super::AppVersion;

#[test]
fn an_unset_app_version_reads_empty() {
    assert_eq!(AppVersion::default().get(), "");
}

#[test]
fn a_clone_taken_before_set_observes_the_value() {
    let version = AppVersion::default();
    let early_clone = version.clone();

    version.set("1.4.2".to_string());

    assert_eq!(early_clone.get(), "1.4.2");
    assert_eq!(version.get(), "1.4.2");
}
