// Tests extracted from `crates/demeteo-core/src/domain/discovery_host.rs` (mirrored-tests convention). `super` = that module.

use super::*;

/// The bug this rule exists to have failed on: a picker whose value the row
/// silently replaced with the project's host.
#[test]
fn a_chosen_machine_wins_over_the_projects_own() {
    let picked = interviewer_machine(Some("runner-01"), true, None).unwrap();
    assert_eq!(picked.as_str(), "runner-01");

    let picked = interviewer_machine(Some("local"), false, Some("runner-02")).unwrap();
    assert_eq!(picked.as_str(), LOCAL_MACHINE);
}

/// A picker the user never touched sends a blank value, not an absent one.
#[test]
fn a_blank_choice_is_no_choice_at_all() {
    let picked = interviewer_machine(Some("   "), false, Some("runner-02")).unwrap();
    assert_eq!(picked.as_str(), "runner-02");
}

#[test]
fn a_local_project_falls_back_to_the_desktop_host() {
    let picked = interviewer_machine(None, true, None).unwrap();
    assert_eq!(picked.as_str(), LOCAL_MACHINE);
}

/// A remote project with nothing recorded has no host to fall back to, and
/// guessing the desktop's would send the interview to a machine with no clone.
#[test]
fn a_remote_project_with_no_host_refuses_rather_than_guessing() {
    assert!(interviewer_machine(None, false, None).is_err());
    assert!(interviewer_machine(None, false, Some("  ")).is_err());
}
