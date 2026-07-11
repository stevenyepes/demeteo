// Tests extracted from `crates/demeteo-core/src/domain/permission.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn read_only_denies_writes_shell_and_net() {
    let p = StepCapability::ReadOnly.base_profile();
    assert_eq!(p.read_fs, Access::Allow);
    assert_eq!(p.write_fs, Access::Deny);
    assert_eq!(p.execute, Access::Deny);
    assert_eq!(p.network, Access::Deny);
    assert_eq!(StepCapability::ReadOnly.write_scope(), WriteScope::None);
}

#[test]
fn artifacts_allows_write_denies_shell() {
    let p = StepCapability::Artifacts.base_profile();
    assert_eq!(p.write_fs, Access::Allow);
    assert_eq!(p.execute, Access::Deny);
    assert_eq!(
        StepCapability::Artifacts.write_scope(),
        WriteScope::ArtifactsOnly
    );
}

#[test]
fn verify_allows_shell_but_scopes_writes_to_artifacts() {
    let p = StepCapability::Verify.base_profile();
    assert_eq!(p.execute, Access::Allow);
    assert_eq!(p.write_fs, Access::Allow);
    assert_eq!(
        StepCapability::Verify.write_scope(),
        WriteScope::ArtifactsOnly
    );
}

#[test]
fn implement_allows_everything_in_worktree() {
    let p = StepCapability::Implement.base_profile();
    assert_eq!(p.write_fs, Access::Allow);
    assert_eq!(p.execute, Access::Allow);
    assert_eq!(StepCapability::Implement.write_scope(), WriteScope::All);
}

#[test]
fn network_default_off_for_every_capability() {
    for cap in [
        StepCapability::ReadOnly,
        StepCapability::Artifacts,
        StepCapability::Verify,
        StepCapability::Implement,
    ] {
        assert_eq!(cap.base_profile().network, Access::Deny);
    }
}

#[test]
fn overrides_widen_only() {
    // Artifacts gains shell + network when toggled on.
    let p = resolve_profile(StepCapability::Artifacts, true, true);
    assert_eq!(p.network, Access::Allow);
    assert_eq!(p.execute, Access::Allow);
    assert_eq!(p.write_fs, Access::Allow);

    // Toggling off leaves the base posture untouched.
    let p = resolve_profile(StepCapability::Artifacts, false, false);
    assert_eq!(p.network, Access::Deny);
    assert_eq!(p.execute, Access::Deny);
}

#[test]
fn capability_round_trips_through_serde_snake_case() {
    let json = serde_json::to_string(&StepCapability::ReadOnly).unwrap();
    assert_eq!(json, "\"read_only\"");
    let back: StepCapability = serde_json::from_str("\"artifacts\"").unwrap();
    assert_eq!(back, StepCapability::Artifacts);
}
