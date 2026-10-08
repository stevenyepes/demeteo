use super::*;

/// The run this was found on: the terminal push failed on the runner, which
/// marked the run `failed` while its feature stayed `awaiting_mr`.
#[test]
fn a_run_whose_push_failed_may_publish_again() {
    assert_eq!(publish_refusal("failed", "awaiting_mr", None), None);
}

#[test]
fn a_run_parked_for_credentials_may_publish() {
    assert_eq!(
        publish_refusal("needs-credentials", "awaiting_mr", None),
        None
    );
}

#[test]
fn a_run_still_driven_by_a_task_is_refused() {
    for status in ["pending", "running"] {
        assert_eq!(
            publish_refusal(status, "awaiting_mr", None),
            Some(PublishRefusal::InFlight {
                status: status.to_string()
            }),
        );
    }
}

#[test]
fn a_feature_that_did_not_finish_has_nothing_to_publish() {
    for feature_status in ["failed", "interrupted", "cancelled", "running"] {
        assert_eq!(
            publish_refusal("failed", feature_status, None),
            Some(PublishRefusal::Unfinished {
                feature_status: feature_status.to_string()
            }),
        );
    }
}

#[test]
fn a_feature_with_a_pr_is_not_pushed_again() {
    assert_eq!(
        publish_refusal(
            "pr_ready",
            "completed",
            Some("https://example.invalid/pr/1")
        ),
        Some(PublishRefusal::AlreadyOpen {
            url: "https://example.invalid/pr/1".to_string()
        }),
    );
    assert_eq!(publish_refusal("failed", "completed", Some("")), None);
}
