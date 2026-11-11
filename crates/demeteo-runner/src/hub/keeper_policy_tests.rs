// Tests extracted from `crates/demeteo-runner/src/hub/keeper_policy.rs` (mirrored-tests convention). `super` = that module.

use super::{watch, Answer, Attempts, Step, StopReason, Timing, Watch};
use std::time::Duration;

const T: Timing = Timing::PRODUCTION;
const NOW_MS: u64 = 1_800_000_000_000;

fn refused(status: u16) -> Answer {
    Answer::Refused { status }
}

#[test]
fn a_token_is_replaced_ten_minutes_before_it_expires() {
    let expires_at = NOW_MS / 1000 + 3600;
    assert_eq!(
        Attempts::default().after(Answer::Token { expires_at }, NOW_MS, &T),
        Step::Hold {
            refresh_at_ms: (expires_at - 600) * 1000
        }
    );
}

#[test]
fn a_token_already_inside_the_margin_is_still_held_for_a_minute() {
    for expires_at in [NOW_MS / 1000 + 60, NOW_MS / 1000 - 5, 0] {
        assert_eq!(
            Attempts::default().after(Answer::Token { expires_at }, NOW_MS, &T),
            Step::Hold {
                refresh_at_ms: NOW_MS + 60_000
            },
            "expires_at {expires_at}"
        );
    }
}

#[test]
fn transient_answers_back_off_from_two_seconds_to_a_one_minute_cap() {
    let mut attempts = Attempts::default();
    let waits: Vec<_> = [
        Answer::Unavailable,
        refused(503),
        refused(500),
        refused(401),
        refused(429),
        Answer::Unavailable,
        Answer::Unavailable,
    ]
    .into_iter()
    .map(|answer| match attempts.after(answer, NOW_MS, &T) {
        Step::Retry { after } => after.as_secs(),
        other => panic!("{answer:?} should retry, not {other:?}"),
    })
    .collect();
    assert_eq!(waits, [2, 4, 8, 16, 32, 60, 60]);
}

#[test]
fn a_token_resets_the_backoff() {
    let mut attempts = Attempts::default();
    for _ in 0..4 {
        attempts.after(Answer::Unavailable, NOW_MS, &T);
    }
    attempts.after(
        Answer::Token {
            expires_at: NOW_MS / 1000 + 3600,
        },
        NOW_MS,
        &T,
    );
    assert_eq!(
        attempts.after(Answer::Unavailable, NOW_MS, &T),
        Step::Retry {
            after: Duration::from_secs(2)
        }
    );
}

#[test]
fn not_yet_accepted_is_retried_until_its_budget_runs_out() {
    let mut attempts = Attempts::default();
    let budget_ms = T.not_yet_budget.as_millis() as u64;
    assert!(matches!(
        attempts.after(refused(404), NOW_MS, &T),
        Step::Retry { .. }
    ));
    assert!(matches!(
        attempts.after(refused(404), NOW_MS + budget_ms - 1, &T),
        Step::Retry { .. }
    ));
    assert_eq!(
        attempts.after(refused(404), NOW_MS + budget_ms, &T),
        Step::Stop(StopReason::NeverAccepted)
    );
}

#[test]
fn the_not_yet_budget_restarts_after_a_token() {
    let mut attempts = Attempts::default();
    let budget_ms = T.not_yet_budget.as_millis() as u64;
    attempts.after(refused(404), NOW_MS, &T);
    attempts.after(
        Answer::Token {
            expires_at: NOW_MS / 1000 + 3600,
        },
        NOW_MS + 1,
        &T,
    );
    assert!(matches!(
        attempts.after(refused(404), NOW_MS + budget_ms, &T),
        Step::Retry { .. }
    ));
}

#[test]
fn the_terminal_answers_stop_the_keeper() {
    for (status, reason) in [
        (409, StopReason::FeatureMismatch),
        (410, StopReason::RunEnded),
        (422, StopReason::NotGitHub),
        (400, StopReason::Refused(400)),
        (403, StopReason::Refused(403)),
    ] {
        assert_eq!(
            Attempts::default().after(refused(status), NOW_MS, &T),
            Step::Stop(reason),
            "HTTP {status}"
        );
    }
}

#[test]
fn a_retried_run_waits_out_ended_until_its_budget_or_first_token() {
    let budget_ms = T.not_yet_budget.as_millis() as u64;
    let mut attempts = Attempts::reopened();
    assert!(matches!(
        attempts.after(refused(410), NOW_MS, &T),
        Step::Retry { .. }
    ));
    assert_eq!(
        attempts.after(refused(410), NOW_MS + budget_ms, &T),
        Step::Stop(StopReason::RunEnded)
    );

    let mut attempts = Attempts::reopened();
    attempts.after(refused(410), NOW_MS, &T);
    attempts.after(
        Answer::Token {
            expires_at: NOW_MS / 1000 + 3600,
        },
        NOW_MS + 1,
        &T,
    );
    assert_eq!(
        attempts.after(refused(410), NOW_MS + 2, &T),
        Step::Stop(StopReason::RunEnded)
    );
}

#[test]
fn a_run_is_watched_until_it_leaves_the_statuses_that_can_use_a_credential() {
    for (status, holds, expected) in [
        (Some("pending"), false, Watch::Continue),
        (Some("running"), true, Watch::Continue),
        (Some("needs-credentials"), false, Watch::Continue),
        (Some("needs-credentials"), true, Watch::Nudge),
        (Some("pr_ready"), true, Watch::End),
        (Some("awaiting_mr"), true, Watch::End),
        (Some("completed"), false, Watch::End),
        (Some("failed"), false, Watch::End),
        (Some("cancelled"), false, Watch::End),
        (Some("over-budget"), false, Watch::End),
        (Some("interrupted"), true, Watch::End),
        (None, false, Watch::End),
    ] {
        assert_eq!(watch(status, holds), expected, "{status:?} holding {holds}");
    }
}
