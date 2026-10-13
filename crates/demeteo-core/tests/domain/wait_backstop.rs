use super::*;

const HOUR: Duration = Duration::from_secs(60 * 60);

#[test]
fn a_run_that_never_ends_is_given_up_on_after_the_limit() {
    let mut backstop = WaitBackstop::default();

    assert!(!backstop.observe(MAX_UNATTENDED_WAIT, false));
    assert!(backstop.observe(Duration::from_secs(1), false));
}

/// The incident's sibling: four days at a ship gate with no restart at all.
#[test]
fn days_spent_waiting_on_a_person_never_trip_it() {
    let mut backstop = WaitBackstop::default();

    for _ in 0..4 * 24 {
        assert!(!backstop.observe(HOUR, true));
    }
}

#[test]
fn time_at_a_gate_is_not_counted_against_the_work_around_it() {
    let mut backstop = WaitBackstop::default();

    assert!(!backstop.observe(20 * HOUR, false));
    assert!(!backstop.observe(72 * HOUR, true));
    assert!(!backstop.observe(4 * HOUR, false));
    assert!(backstop.observe(HOUR, false));
}
