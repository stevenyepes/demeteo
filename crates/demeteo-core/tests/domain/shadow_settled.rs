use super::*;

/// The incident: the runner parked the run row `failed` while its driver went
/// on to run the last step.
#[test]
fn a_terminal_run_whose_feature_is_still_moving_is_not_settled() {
    for feature in [
        "running",
        "awaiting_gate",
        "gated",
        "bootstrapping",
        "pending",
    ] {
        assert!(!shadow_settled("failed", Some(feature)), "{feature}");
    }
}

#[test]
fn a_terminal_run_over_a_finished_feature_is_settled() {
    for run in ["failed", "cancelled", "awaiting_mr", "completed"] {
        for feature in ["failed", "cancelled", "awaiting_mr", "completed"] {
            assert!(shadow_settled(run, Some(feature)), "{run} over {feature}");
        }
    }
}

#[test]
fn a_run_still_in_flight_is_never_settled() {
    for run in [
        "pending",
        "running",
        "parked",
        "needs-credentials",
        "interrupted",
        "unreachable",
    ] {
        assert!(!shadow_settled(run, Some("completed")), "{run}");
        assert!(!shadow_settled(run, None), "{run}");
    }
}

#[test]
fn a_run_that_never_made_a_feature_settles_on_its_row() {
    assert!(shadow_settled("failed", None));
}
