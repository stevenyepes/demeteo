// Tests extracted from `crates/demeteo-runner/src/rpc/mod.rs` (mirrored-tests convention). `super` = that module.

use super::health_info;

fn health_json() -> serde_json::Value {
    serde_json::to_value(health_info()).expect("HealthInfo serialises")
}

#[test]
fn health_reports_build_version_as_the_runner_version() {
    assert_eq!(health_json()["build_version"], crate::VERSION);
}

#[test]
fn health_keeps_the_crate_version_key() {
    assert_eq!(health_json()["version"], env!("CARGO_PKG_VERSION"));
}
