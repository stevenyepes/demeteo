// Tests extracted from `crates/demeteo-core/src/domain/models/feature.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn launched_by_round_trips_through_its_column() {
    for who in [LaunchedBy::Desktop, LaunchedBy::Hub] {
        assert_eq!(LaunchedBy::from_column(who.to_column()), who);
    }
}

#[test]
fn a_desktop_launch_stores_as_null() {
    assert_eq!(LaunchedBy::Desktop.to_column(), None);
    assert_eq!(LaunchedBy::Hub.to_column(), Some("hub"));
}

#[test]
fn an_unknown_column_value_reads_as_desktop() {
    assert_eq!(LaunchedBy::from_column(Some("phone")), LaunchedBy::Desktop);
    assert_eq!(LaunchedBy::from_column(Some("")), LaunchedBy::Desktop);
}

#[test]
fn a_feature_serialized_before_launched_by_existed_reads_as_desktop() {
    let json = serde_json::json!({
        "id": "f-1",
        "project_id": "p-1",
        "workflow_id": null,
        "title": "t",
        "status": "running",
        "total_cost": 0.0,
        "duration": "0s",
        "created_at": 0,
    });
    let feature: Feature = serde_json::from_value(json).expect("pre-field Feature deserializes");
    assert_eq!(feature.launched_by, LaunchedBy::Desktop);
}
