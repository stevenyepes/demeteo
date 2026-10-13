use super::*;

const STARTED: i64 = 1_000_000;

/// The incident: a run four days at its ship gate, across six restarts.
#[test]
fn a_question_already_waiting_when_the_process_started_interrupted_nothing() {
    assert!(!restart_interrupted_work(Some(STARTED - 1), STARTED));
}

/// The engine's own reconciliation poses a question for the step it found
/// running, and it does so inside the process that is asking.
#[test]
fn a_question_posed_by_this_process_is_the_restarts_own() {
    assert!(restart_interrupted_work(Some(STARTED), STARTED));
    assert!(restart_interrupted_work(Some(STARTED + 40), STARTED));
}

#[test]
fn no_question_at_all_is_read_as_interrupted_work() {
    assert!(restart_interrupted_work(None, STARTED));
}

#[test]
fn a_restart_that_interrupted_nothing_resumes_without_a_charge() {
    assert_eq!(restart_disposition(0, false), RestartDisposition::Resume);
}

#[test]
fn a_spent_budget_does_not_park_a_run_that_was_only_waiting() {
    for count in [REBOOT_RETRY_BUDGET, REBOOT_RETRY_BUDGET + 3] {
        assert_eq!(
            restart_disposition(count, false),
            RestartDisposition::Resume
        );
    }
}

#[test]
fn interrupted_work_is_charged_until_the_budget_is_spent() {
    for count in 0..REBOOT_RETRY_BUDGET {
        assert_eq!(
            restart_disposition(count, true),
            RestartDisposition::ResumeCharged,
            "count {count}"
        );
    }
    assert_eq!(
        restart_disposition(REBOOT_RETRY_BUDGET, true),
        RestartDisposition::Park
    );
}

#[test]
fn only_a_run_a_task_was_driving_is_reconciled() {
    for status in ["running", "pending", "interrupted"] {
        assert!(left_unfinished_by_restart(status), "{status}");
    }
    for status in [
        "failed",
        "cancelled",
        "pr_ready",
        "awaiting_mr",
        "completed",
        "needs-credentials",
        "over-budget",
    ] {
        assert!(!left_unfinished_by_restart(status), "{status}");
    }
}
