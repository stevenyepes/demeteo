//! Whether an agent failure is the provider refusing the account, not the
//! work going wrong. See [`crate::domain`].
//!
//! A harness that hit its plan's usage limit, its rate limit, or the end of
//! its credits fails every turn identically until the quota resets — minutes
//! to days away. Classified as an ordinary `agent_failure`, it was answered
//! with the step's `on_failure` redirect, and the whole retry budget went on
//! re-sending a call that could not succeed. The step executor reads this
//! before the retry policy and ends the step as `usage_limit`, which no rule
//! retries.
//!
//! Detection is by the provider's own wording, matched on the error text
//! that reached the step. Only failures that are not a verdict are read: a
//! verdict carries the project's test output, where "rate limit exceeded" can
//! be the name of a test rather than the provider talking.
//!
//! No retry-after timer is derived from the reset time. The text names a
//! wall-clock time in the provider's own format and zone, sometimes without
//! a date; it is quoted to the reader, never parsed into a schedule.

/// Phrases a provider's usage, rate or credit limit is reported with, ASCII
/// lower-cased. Each names the account's limit rather than the work: a phrase
/// as broad as "limit" alone would catch an agent reporting a test about one.
///
/// Apostrophes are left out of every phrase — codex writes a curly one
/// ("You’ve hit your usage limit"), and a straight-quote phrase would miss it.
const MARKERS: &[&str] = &[
    // codex (ChatGPT plan): "You’ve hit your usage limit. Upgrade to Pro …
    // or try again at 7:26 PM."
    "hit your usage limit",
    // Claude Code (subscription): "You've hit your limit · resets 3pm
    // (America/New_York)", "Claude AI usage limit reached|<epoch>",
    // "Claude usage limit reached. Your limit will reset at 7pm (…)."
    "hit your limit",
    "usage limit reached",
    "5-hour limit reached",
    "weekly limit reached",
    // Anthropic API, through Claude Code: the out-of-credit answer.
    "credit balance is too low",
    // OpenAI API (codex on an API key, hermes, pi): the `insufficient_quota`
    // error code and its message.
    "insufficient_quota",
    "exceeded your current quota",
    // Not here, on purpose: a bare HTTP 429 or Anthropic's `rate_limit_error`.
    // Those are usually per-minute limits that lift before the next attempt,
    // so ending the step on them would throw away a run a retry would save.
    // opencode: the raw error names, and the "OpenCode Rate/Credit Limit
    // Exceeded" text `format_agent_error_message` rewrites them into.
    "freeusagelimiterror",
    "creditlimiterror",
    "rate limit exceeded",
    "credit limit exceeded",
];

/// Where a provider says when the limit lifts, ASCII lower-cased. Longest
/// first, so "resets at" is not read as "resets" followed by "at …".
const RESET_MARKERS: &[&str] = &["try again at ", "resets at ", "reset at ", "resets "];

/// The reset phrase is quoted, not rendered, so it is bounded: a marker that
/// matched inside a long log line must not drag the rest of it along.
const RESET_HINT_MAX_CHARS: usize = 80;

/// A provider refused the call because the account is over a limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageLimit {
    /// The provider's own words for when it lifts, verbatim from the reset
    /// marker to the end of its clause — "try again at 7:26 PM", "resets 3pm
    /// (America/New_York)". `None` when the text names no time.
    pub reset_hint: Option<String>,
}

/// `Some` when `error` is a provider usage, rate or credit limit.
pub fn detect(error: &str) -> Option<UsageLimit> {
    // ASCII lower-casing keeps every byte offset, so an index found in
    // `lower` slices `error` on the same character boundary.
    let lower = error.to_ascii_lowercase();
    if !MARKERS.iter().any(|m| lower.contains(m)) {
        return None;
    }
    Some(UsageLimit {
        reset_hint: reset_hint(error, &lower),
    })
}

fn reset_hint(error: &str, lower: &str) -> Option<String> {
    let (start, marker) = RESET_MARKERS
        .iter()
        .find_map(|m| lower.find(m).map(|i| (i, m.len())))?;
    let clause = &error[start..];
    let clause = clause[..clause_end(clause)].trim_end();
    // A marker with nothing after it ("try again at ") names no time.
    if clause.len() <= marker {
        return None;
    }
    Some(clause.chars().take(RESET_HINT_MAX_CHARS).collect())
}

/// The clause ends at a line break, a sentence stop, or the `|<epoch>` suffix
/// older Claude Code builds append. A sentence stop is a `.` followed by
/// whitespace or the end, so the `.` inside a URL or "a.m." does not cut it.
fn clause_end(clause: &str) -> usize {
    let bytes = clause.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\n' | b'\r' | b'|' => return i,
            b'.' if bytes.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()) => return i,
            _ => {}
        }
    }
    clause.len()
}

/// The step's failure text: what happened, why it was not retried, when it
/// may succeed if the provider said, and the original error for the rest.
pub fn terminal_message(limit: &UsageLimit, error: &str) -> String {
    let when = match &limit.reset_hint {
        Some(hint) => format!(" The provider says: \"{hint}\"."),
        None => String::new(),
    };
    format!(
        "The agent's harness hit its provider usage limit, so this step was not retried: \
         no attempt can succeed until the limit resets.{when} Provider error: {error}"
    )
}

#[cfg(test)]
#[path = "../../tests/domain/usage_limit.rs"]
mod tests;
