// Tests extracted from `crates/demeteo-core/src/domain/run_placement.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::LOCAL_MACHINE;

fn id(s: &str) -> MachineId {
    MachineId::from(s)
}

fn detached(s: &str) -> RunPlacement {
    RunPlacement::Detached { machine_id: id(s) }
}

fn machine(machine_id: &str, auth_type: &str) -> Machine {
    Machine {
        id: id(machine_id),
        name: machine_id.to_string(),
        host: "10.0.0.7".to_string(),
        port: 22,
        username: "dev".to_string(),
        auth_type: auth_type.to_string(),
        key_path: None,
        agents: None,
        auto_approved_rules: None,
        use_login_shell: None,
        setup_commands: None,
        notify_webhook_url: None,
    }
}

#[test]
fn the_wire_shape_is_tagged_by_kind() {
    assert_eq!(
        serde_json::to_value(RunPlacement::Local).unwrap(),
        serde_json::json!({ "kind": "local" })
    );
    assert_eq!(
        serde_json::to_value(detached("runner-01")).unwrap(),
        serde_json::json!({ "kind": "detached", "machine_id": "runner-01" })
    );
}

#[test]
fn both_spellings_of_the_desktop_are_local() {
    assert_eq!(placement_for(&id(LOCAL_MACHINE)), RunPlacement::Local);
    assert_eq!(placement_for(&id("")), RunPlacement::Local);
    assert_eq!(placement_for(&id("runner-01")), detached("runner-01"));
}

// D4

#[test]
fn a_local_discovery_defaults_local() {
    assert_eq!(
        default_ticket_placement(&id(LOCAL_MACHINE), None),
        RunPlacement::Local
    );
    assert_eq!(
        default_ticket_placement(&id(LOCAL_MACHINE), Some(&id("runner-01"))),
        RunPlacement::Local
    );
}

/// The case the module doc records: these tickets already run on that host,
/// attached, and must not silently become detached.
#[test]
fn a_discovery_on_the_projects_own_host_defaults_local() {
    assert_eq!(
        default_ticket_placement(&id("runner-01"), Some(&id("runner-01"))),
        RunPlacement::Local
    );
}

#[test]
fn a_discovery_on_another_machine_than_the_projects_defaults_detached() {
    assert_eq!(
        default_ticket_placement(&id("runner-02"), Some(&id("runner-01"))),
        detached("runner-02")
    );
}

#[test]
fn a_local_project_with_a_remote_discovery_defaults_detached() {
    assert_eq!(
        default_ticket_placement(&id("runner-02"), None),
        detached("runner-02")
    );
}

// D5

#[test]
fn nothing_chosen_inherits_the_default() {
    let resolved = ticket_placement(None, None, detached("runner-02"));
    assert_eq!(resolved.placement, detached("runner-02"));
    assert!(resolved.inherited);
}

#[test]
fn a_stored_machine_beats_the_default() {
    let resolved = ticket_placement(None, Some(&id("runner-03")), detached("runner-02"));
    assert_eq!(resolved.placement, detached("runner-03"));
    assert!(!resolved.inherited);
}

/// `"local"` stored is a choice, not an absence: it opts a ticket out of a
/// detached default.
#[test]
fn a_stored_local_beats_a_remote_default() {
    let resolved = ticket_placement(None, Some(&id(LOCAL_MACHINE)), detached("runner-02"));
    assert_eq!(resolved.placement, RunPlacement::Local);
    assert!(!resolved.inherited);
}

#[test]
fn an_override_beats_the_stored_choice() {
    let resolved = ticket_placement(
        Some(&id("runner-04")),
        Some(&id("runner-03")),
        RunPlacement::Local,
    );
    assert_eq!(resolved.placement, detached("runner-04"));
    assert!(!resolved.inherited);
}

#[test]
fn a_local_override_beats_a_detached_default() {
    let resolved = ticket_placement(Some(&id(LOCAL_MACHINE)), None, detached("runner-02"));
    assert_eq!(resolved.placement, RunPlacement::Local);
    assert!(!resolved.inherited);
}

#[test]
fn an_absent_override_is_no_override() {
    assert_eq!(placement_override(None), None);
}

/// A blank id would otherwise reach [`MachineId::is_local`], which accepts
/// empty, and override a stored detached choice to local.
#[test]
fn a_blank_or_whitespace_override_is_no_override() {
    assert_eq!(placement_override(Some("")), None);
    assert_eq!(placement_override(Some("  ")), None);
    let stored = id("runner-03");
    let resolved = ticket_placement(
        placement_override(Some(" ")).as_ref(),
        Some(&stored),
        RunPlacement::Local,
    );
    assert_eq!(resolved.placement, detached("runner-03"));
}

#[test]
fn a_local_override_is_kept() {
    assert_eq!(
        placement_override(Some(LOCAL_MACHINE)),
        Some(id(LOCAL_MACHINE))
    );
}

#[test]
fn a_padded_override_is_trimmed() {
    assert_eq!(placement_override(Some(" m1 ")), Some(id("m1")));
}

/// Unlike [`placement_override`], a blank launch id is not "no choice" — a
/// Feature launch has nothing to inherit, so it is local.
#[test]
fn an_absent_or_blank_launch_machine_is_local() {
    for raw in [None, Some(""), Some("   ")] {
        assert_eq!(placement_from_raw(raw), RunPlacement::Local, "{raw:?}");
    }
}

#[test]
fn a_local_launch_machine_is_local_padded_or_not() {
    for raw in ["local", " local "] {
        assert_eq!(
            placement_from_raw(Some(raw)),
            RunPlacement::Local,
            "{raw:?}"
        );
    }
}

#[test]
fn a_padded_launch_machine_is_detached_on_the_trimmed_id() {
    assert_eq!(
        placement_from_raw(Some(" build-box ")),
        detached("build-box")
    );
}

// D6

#[test]
fn local_with_no_detached_options_passes() {
    assert_eq!(
        check_options(&RunPlacement::Local, &DetachedOptions::default()),
        Ok(())
    );
}

#[test]
fn local_refuses_each_detached_only_option() {
    let cases = [
        (
            "target_repo_id",
            DetachedOptions {
                target_repo_id: Some("repo-1".into()),
                ..Default::default()
            },
        ),
        (
            "unattended",
            DetachedOptions {
                unattended: Some(true),
                ..Default::default()
            },
        ),
        (
            "max_cost_usd",
            DetachedOptions {
                max_cost_usd: Some(5.0),
                ..Default::default()
            },
        ),
        (
            "max_wall_clock_secs",
            DetachedOptions {
                max_wall_clock_secs: Some(600),
                ..Default::default()
            },
        ),
    ];
    for (name, opts) in cases {
        let err = check_options(&RunPlacement::Local, &opts)
            .expect_err(&format!("local + {name} must be refused"));
        assert!(err.contains(name), "{name}: {err}");
    }
}

#[test]
fn detached_accepts_valid_options() {
    let opts = DetachedOptions {
        target_repo_id: Some("repo-1".into()),
        unattended: Some(true),
        max_cost_usd: Some(2.5),
        max_wall_clock_secs: Some(3600),
    };
    assert_eq!(check_options(&detached("runner-01"), &opts), Ok(()));
    assert_eq!(
        check_options(&detached("runner-01"), &DetachedOptions::default()),
        Ok(())
    );
}

#[test]
fn detached_refuses_an_attended_run() {
    let opts = DetachedOptions {
        unattended: Some(false),
        ..Default::default()
    };
    let err = check_options(&detached("runner-01"), &opts).unwrap_err();
    assert!(err.contains("unattended"), "{err}");
}

#[test]
fn detached_refuses_a_cost_cap_that_is_not_a_positive_finite_amount() {
    for cap in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let opts = DetachedOptions {
            max_cost_usd: Some(cap),
            ..Default::default()
        };
        let err = check_options(&detached("runner-01"), &opts)
            .expect_err(&format!("cap {cap} must be refused"));
        assert!(err.contains("max_cost_usd"), "{err}");
    }
}

#[test]
fn detached_refuses_a_zero_wall_clock_cap() {
    let opts = DetachedOptions {
        max_wall_clock_secs: Some(0),
        ..Default::default()
    };
    let err = check_options(&detached("runner-01"), &opts).unwrap_err();
    assert!(err.contains("max_wall_clock_secs"), "{err}");
}

// D7

#[test]
fn a_registered_remote_machine_is_a_valid_target() {
    let row = machine("runner-01", "key");
    assert_eq!(detached_target_refusal(&id("runner-01"), Some(&row)), None);
}

#[test]
fn a_missing_row_names_the_id_and_points_to_machines_settings() {
    let msg = detached_target_refusal(&id("runner-gone"), None).unwrap();
    assert!(msg.contains("runner-gone"), "{msg}");
    assert!(msg.contains("Machines settings"), "{msg}");
}

#[test]
fn the_local_id_is_refused_as_a_detached_target() {
    let msg = detached_target_refusal(&id(LOCAL_MACHINE), None).unwrap();
    assert!(msg.contains("this desktop"), "{msg}");
    assert!(msg.contains("`local`"), "{msg}");
}

#[test]
fn an_empty_id_is_refused_as_a_detached_target() {
    let msg = detached_target_refusal(&id(""), None).unwrap();
    assert!(msg.contains("this desktop"), "{msg}");
    assert!(msg.contains("(empty)"), "{msg}");
}

/// A row can exist for the desktop itself; its id is not `"local"`, so only
/// the row's `auth_type` says so.
#[test]
fn a_local_auth_row_is_refused_as_a_detached_target() {
    let row = machine("desktop-row", "local");
    let msg = detached_target_refusal(&id("desktop-row"), Some(&row)).unwrap();
    assert!(msg.contains("desktop-row"), "{msg}");
    assert!(msg.contains("this desktop"), "{msg}");
}
