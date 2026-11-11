//! What a Hub-started run's credential keeper does next, decided without I/O
//! (AGENTS.md §3, *Where a decision is allowed to live*). The keeper in
//! `keeper.rs` only performs it.
//!
//! The Hub's `/git-token` answers (decision 71): `200` a token; `404` a start
//! it has not yet seen accepted, which is the expected first answer, because
//! the runner asks from inside its launch, before the accepted result has left;
//! `409` a feature that is not that start's; `410` a run that ended; `422` a
//! repository that is not on GitHub; `503` a Hub that cannot mint right now.
//! A `401` has already earned its one refresh inside the fetcher.

use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    /// The first retry's wait; each failure in a row doubles it.
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
    /// How long before expiry a held token is replaced. GitHub's last an
    /// hour, so ten minutes leaves a slow Hub several retries.
    pub refresh_margin: Duration,
    /// The shortest a token is held before the next ask, so a token issued
    /// inside the margin is not asked for again in a loop.
    pub min_hold: Duration,
    /// How long `404` is waited out. The accepted result rides the same
    /// socket the start came on, so it is seconds behind unless that socket
    /// dropped; past this, the start is taken as never accepted.
    pub not_yet_budget: Duration,
    /// How often the run's own status is read while waiting, which is how
    /// the keeper stops with its run and wakes a run parked for want of the
    /// token it holds.
    pub status_poll: Duration,
}

impl Timing {
    pub const PRODUCTION: Self = Self {
        backoff_base: Duration::from_secs(2),
        backoff_cap: Duration::from_secs(60),
        refresh_margin: Duration::from_secs(10 * 60),
        min_hold: Duration::from_secs(60),
        not_yet_budget: Duration::from_secs(30 * 60),
        status_poll: Duration::from_secs(30),
    };
}

/// A fetch's outcome, reduced to what the decision reads. Never the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Unix seconds, by the Hub's clock.
    Token {
        expires_at: u64,
    },
    Refused {
        status: u16,
    },
    /// No answer, or one that did not parse.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Store the token and ask again at `refresh_at_ms`.
    Hold {
        refresh_at_ms: u64,
    },
    Retry {
        after: Duration,
    },
    Stop(StopReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    FeatureMismatch,
    RunEnded,
    NotGitHub,
    NeverAccepted,
    /// A status the contract has no meaning for, which a retry will not fix.
    Refused(u16),
}

impl StopReason {
    /// For the runner's log: a status class, never a body.
    pub fn note(self) -> String {
        match self {
            Self::FeatureMismatch => "the Hub says the feature is not its start's (409)".into(),
            Self::RunEnded => "the Hub says the run ended (410)".into(),
            Self::NotGitHub => "the run's repository is not on GitHub (422)".into(),
            Self::NeverAccepted => "the Hub never saw the start accepted (404)".into(),
            Self::Refused(status) => format!("the Hub refused the git token (HTTP {status})"),
        }
    }
}

/// The failures in a row since the last token, and when the current run of
/// `404`s began.
#[derive(Debug, Default)]
pub(crate) struct Attempts {
    failures: u32,
    not_yet_since_ms: Option<u64>,
    /// Set for a run the Hub retried, until its first token: `410` is then
    /// waited out within the `404` budget, not stopped on. The Hub reads a
    /// run as live again only once the `feature_started` its retry wrote has
    /// crossed the socket, and the keeper asks as the retry is accepted, so
    /// its first answer can still be the end it is retrying from.
    reopened: bool,
}

impl Attempts {
    pub fn reopened() -> Self {
        Self {
            reopened: true,
            ..Self::default()
        }
    }

    pub fn after(&mut self, answer: Answer, now_ms: u64, timing: &Timing) -> Step {
        match answer {
            Answer::Token { expires_at } => {
                self.failures = 0;
                self.not_yet_since_ms = None;
                self.reopened = false;
                let replace_at = expires_at
                    .saturating_mul(1000)
                    .saturating_sub(millis(timing.refresh_margin));
                let earliest = now_ms.saturating_add(millis(timing.min_hold));
                Step::Hold {
                    refresh_at_ms: replace_at.max(earliest),
                }
            }
            Answer::Refused { status: 404 } => {
                let since = *self.not_yet_since_ms.get_or_insert(now_ms);
                if now_ms.saturating_sub(since) >= millis(timing.not_yet_budget) {
                    return Step::Stop(StopReason::NeverAccepted);
                }
                self.retry(timing)
            }
            Answer::Refused { status: 409 } => Step::Stop(StopReason::FeatureMismatch),
            Answer::Refused { status: 410 } if self.reopened => {
                let since = *self.not_yet_since_ms.get_or_insert(now_ms);
                if now_ms.saturating_sub(since) >= millis(timing.not_yet_budget) {
                    return Step::Stop(StopReason::RunEnded);
                }
                self.retry(timing)
            }
            Answer::Refused { status: 410 } => Step::Stop(StopReason::RunEnded),
            Answer::Refused { status: 422 } => Step::Stop(StopReason::NotGitHub),
            // A 401 that survived its refresh: the pairing may come back.
            Answer::Refused { status: 401 | 429 } | Answer::Unavailable => self.retry(timing),
            Answer::Refused { status } if status >= 500 => self.retry(timing),
            Answer::Refused { status } => Step::Stop(StopReason::Refused(status)),
        }
    }

    fn retry(&mut self, timing: &Timing) -> Step {
        let doubled = timing
            .backoff_base
            .saturating_mul(2u32.saturating_pow(self.failures.min(16)));
        self.failures = self.failures.saturating_add(1);
        Step::Retry {
            after: doubled.min(timing.backoff_cap),
        }
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// What the keeper does on reading its run's status between asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Watch {
    Continue,
    /// The run parked `needs-credentials` while a token is held: re-drive
    /// it, as `inject_credentials` does.
    Nudge,
    /// The run is over, or gone: wipe and stop.
    End,
}

/// Only `pending`, `running` and `needs-credentials` are a run still able to
/// use a credential. Every other status is one the driver wiped on, or the
/// SIGTERM mark of a process about to exit, whose restart re-reads it.
pub(crate) fn watch(status: Option<&str>, holds_credential: bool) -> Watch {
    match status {
        Some("pending" | "running") => Watch::Continue,
        Some("needs-credentials") if holds_credential => Watch::Nudge,
        Some("needs-credentials") => Watch::Continue,
        _ => Watch::End,
    }
}

#[cfg(test)]
#[path = "keeper_policy_tests.rs"]
mod tests;
