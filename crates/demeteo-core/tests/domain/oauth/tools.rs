// domain::oauth::tools's own vocabulary. `super` = `domain::oauth::tools`.

use super::*;

#[test]
fn every_declared_tool_maps_to_its_exact_tier() {
    let cases = [
        ("list_projects", Scope::Read),
        ("list_features", Scope::Read),
        ("get_feature", Scope::Read),
        ("list_step_attempts", Scope::Read),
        ("get_failure_verdict", Scope::Read),
        ("list_pending_gates", Scope::Read),
        ("get_discovery_board", Scope::Read),
        ("run_events_since", Scope::Read),
        ("create_workspace_project", Scope::Configure),
        ("apply_run_shape_patch", Scope::Configure),
        ("start_feature", Scope::Spend),
        ("start_ticket", Scope::Spend),
    ];

    for (name, expected) in cases {
        assert_eq!(
            required_scope(name),
            Some(expected),
            "tool {name} should map to {expected:?}"
        );
    }
}

#[test]
fn unrecognized_tool_name_has_no_scope() {
    assert_eq!(required_scope("delete_everything"), None);
}

#[test]
fn only_server_discover_is_open_and_the_handshake_needs_a_session() {
    assert_eq!(method_auth("server/discover"), MethodAuth::Open);
    assert_eq!(method_auth("tools/call"), MethodAuth::PerTool);
    for method in [
        "initialize",
        "tools/list",
        "notifications/initialized",
        "unknown/method",
    ] {
        assert_eq!(method_auth(method), MethodAuth::Session, "{method}");
    }
}
