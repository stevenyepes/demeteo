// Tests for `src/domain/usage_limit.rs` (mirrored-tests convention).
//
// Every string here is shaped like what reached a step's failure text in a
// real run — wrapped in the context the sequence step adds — because the
// detector reads that text, not the harness's raw event.

use super::*;

const CODEX: &str = "sequence task 'instance-metadata-projection': agent error: \
    You\u{2019}ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), \
    visit https://chatgpt.com/settings/usage to purchase more credits or try again at 7:26 PM.";

#[test]
fn codex_usage_limit_with_a_curly_apostrophe_is_detected_with_its_reset_time() {
    let hit = detect(CODEX).expect("codex usage limit");
    assert_eq!(hit.reset_hint.as_deref(), Some("try again at 7:26 PM"));
}

#[test]
fn claude_code_subscription_limits_are_detected() {
    let cases = [
        (
            "agent error: You've hit your limit \u{b7} resets 3pm (America/New_York)",
            Some("resets 3pm (America/New_York)"),
        ),
        (
            "agent error: Claude usage limit reached. Your limit will reset at 7pm (Europe/Paris).",
            Some("reset at 7pm (Europe/Paris)"),
        ),
        (
            "agent error: Claude AI usage limit reached|1736802000",
            None,
        ),
        (
            "agent error: 5-hour limit reached \u{2219} resets 3am",
            Some("resets 3am"),
        ),
    ];
    for (text, hint) in cases {
        let hit = detect(text).unwrap_or_else(|| panic!("not detected: {text}"));
        assert_eq!(hit.reset_hint.as_deref(), hint, "{text}");
    }
}

#[test]
fn api_rate_limit_and_quota_errors_are_detected() {
    for text in [
        "agent error: Your credit balance is too low to access the Anthropic API.",
        "agent error: You exceeded your current quota, please check your plan and billing details.",
        r#"agent error: {"error":{"code":"insufficient_quota"}}"#,
    ] {
        assert!(detect(text).is_some(), "not detected: {text}");
    }
}

/// A per-minute limit lifts before the next attempt, so it keeps the retry.
#[test]
fn transient_rate_limits_are_not_usage_limits() {
    for text in [
        r#"agent error: API Error: 429 {"type":"error","error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit"}}"#,
        "agent error: exceeded retry limit, last status: 429 Too Many Requests",
    ] {
        assert_eq!(detect(text), None, "{text}");
    }
}

#[test]
fn opencode_limits_are_detected_raw_and_as_rewritten() {
    for text in [
        "agent error: FreeUsageLimitError: free tier exhausted",
        "agent error: CreditLimitError",
        "OpenCode Rate Limit Exceeded: The free model rate limit was reached. Please try changing the model to a different model (e.g. 'opencode/big-pickle') or try again later.",
        "OpenCode Credit Limit Exceeded: You have run out of credits or reached your usage quota.",
    ] {
        assert!(detect(text).is_some(), "not detected: {text}");
    }
}

#[test]
fn ordinary_agent_failures_are_not_usage_limits() {
    for text in [
        "sequence task 'snapshot-metadata-protocol': agent error: Agent step exceeded wall clock cap (1800s / 1800s elapsed)",
        "the agent stopped at its turn cap (--max-turns) without reporting back",
        "agent error: limit the scope of the change to src/",
        "declared artifact never produced: plan.md",
    ] {
        assert_eq!(detect(text), None, "{text}");
    }
}

#[test]
fn a_reset_marker_with_no_time_after_it_names_no_reset() {
    let hit = detect("You\u{2019}ve hit your usage limit. Try again at ").unwrap();
    assert_eq!(hit.reset_hint, None);
}

#[test]
fn the_terminal_message_names_the_limit_its_reset_and_the_original_error() {
    let msg = terminal_message(&detect(CODEX).unwrap(), CODEX);
    assert!(msg.contains("hit its provider usage limit"), "{msg}");
    assert!(msg.contains("not retried"), "{msg}");
    assert!(msg.contains("\"try again at 7:26 PM\""), "{msg}");
    assert!(msg.ends_with(CODEX), "{msg}");
}

#[test]
fn the_terminal_message_claims_no_reset_time_the_provider_did_not_give() {
    let text = "agent error: Claude AI usage limit reached|1736802000";
    let msg = terminal_message(&detect(text).unwrap(), text);
    assert!(!msg.contains("The provider says"), "{msg}");
}
