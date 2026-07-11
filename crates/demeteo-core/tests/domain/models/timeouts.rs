// Tests extracted from `crates/demeteo-core/src/domain/models/timeouts.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn default_values_match_documented_thresholds() {
    let t = AgentTimeouts::default();
    assert_eq!(t.fast_timeout_s, 300);
    assert_eq!(t.normal_timeout_s, 600);
    assert_eq!(t.wall_cap_s, 1800);
}

#[test]
fn json_round_trip() {
    let original = AgentTimeouts {
        fast_timeout_s: 120,
        normal_timeout_s: 240,
        wall_cap_s: 900,
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: AgentTimeouts = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, original);
}

#[test]
fn missing_fields_fall_back_to_defaults() {
    let parsed: AgentTimeouts = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed, AgentTimeouts::default());
}

#[test]
fn validated_rejects_out_of_range_fast() {
    assert!(AgentTimeouts::validated(5, 600, 1800).is_err());
    assert!(AgentTimeouts::validated(4000, 600, 1800).is_err());
}

#[test]
fn validated_rejects_normal_less_than_fast() {
    assert!(AgentTimeouts::validated(300, 100, 1800).is_err());
}

#[test]
fn validated_rejects_wall_less_than_normal() {
    assert!(AgentTimeouts::validated(100, 200, 100).is_err());
}

#[test]
fn validated_accepts_monotonic_values() {
    assert!(AgentTimeouts::validated(300, 600, 1800).is_ok());
    assert!(AgentTimeouts::validated(10, 10, 10).is_ok());
}
