//! Why a pull-request listing came back with nothing, in the five shapes the
//! user can act on differently. See [`crate::domain`].
//!
//! A listing has one failure mode that is worse than all the others put
//! together: rendering as an empty list. "Nothing is waiting for review" and
//! "your token expired" are the same picture, and only one of them is a reason
//! to stop looking. So no status outside 2xx may reach the caller as `Ok` —
//! [`classify_list_response`] is the only thing that decides, it is synchronous
//! and pure, and its sibling test walks the statuses that tempt an adapter into
//! coercion.
//!
//! The neighbouring `fetch_mr_state` path does coerce: any `>= 300` answers
//! `open`, because a poll that guesses wrong costs a stale badge. Reading a
//! listing wrong costs the user the queue itself, so the two paths disagree on
//! purpose and this one refuses to guess.
//!
//! ## The wire shape is the contract
//!
//! This enum crosses IPC as its serde form — `{"kind": "unauthorized", …}`,
//! kebab-case, fields flattened alongside the tag — and `src/lib/pullRequests.ts`
//! declares the same five shapes to receive it. That is a deliberate departure
//! from [`crate::error::AppError`], which every other command stringifies to a
//! bare sentence: a sentence cannot carry which host answered, which status it
//! answered with, or how long the rate limit has left, and those are precisely
//! what separates "reconnect your provider" from "try again in a minute".
//! `list_open_pull_requests` therefore serializes this to JSON for its `Err`
//! rather than calling `.to_string()`.
//!
//! Renaming a variant or a field here silently breaks that decoder, so the
//! serialized spelling is pinned by `serialized_shape_is_the_wire_contract` on
//! this side and by `src/lib/pullRequests.test.ts` on the other, both quoting
//! the same literals.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The provider a listing was read from, in the two facts every failure
/// message needs to name it.
#[derive(Debug, Clone, Copy)]
pub struct ListTarget<'a> {
    /// `github` or `gitlab`, as [`crate::domain::models::ProviderInstance`]
    /// spells it.
    pub kind: &'a str,
    pub host: &'a str,
}

/// What a provider answered a list request with.
#[derive(Debug, Clone, Copy)]
pub struct ListResponse<'a> {
    pub status: u16,
    pub body: &'a str,
    pub headers: &'a [(String, String)],
}

/// Reading the open pull requests failed.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MrListError {
    /// No provider instance resolves for the repository, so there is nothing
    /// to ask. Distinct from every other variant in that the user's next move
    /// is connecting one, not retrying.
    #[error("no provider instance is connected for this project")]
    NoProvider,

    /// A provider is connected, but no token for it came out of the keyring,
    /// so the request was never made. The user's next move is the same as for
    /// [`MrListError::Unauthorized`] — reconnect — but the evidence is not, and
    /// that is the whole reason this is its own variant: reported as a status
    /// the provider never issued, a local failure sends the user to audit the
    /// scopes of a token that is fine. `detail` is the keyring's own words,
    /// because an absent entry and an OS refusing to release an existing one
    /// are different problems and nothing else in the tree records which.
    #[error("no {provider} token is stored for {host}: {detail}")]
    NoCredential {
        provider: String,
        host: String,
        detail: String,
    },

    /// The provider refused the token it was given.
    #[error("{host} rejected the {provider} token with HTTP {status}")]
    Unauthorized {
        provider: String,
        host: String,
        status: u16,
    },

    /// The token is over its quota. `retry_after` is the `Retry-After` header
    /// in seconds when the provider sent one in that form; the HTTP-date form
    /// yields `None`, for the reason the private `retry_after_seconds` records.
    #[error("{host} is rate-limiting this token")]
    RateLimited {
        host: String,
        retry_after: Option<u64>,
    },

    /// Everything else, carrying the provider's own words rather than a
    /// summary of them. A status Demeteo has never seen is exactly the case
    /// where paraphrasing costs the user the only evidence they have, so
    /// `body` is verbatim and merely capped.
    #[error("{host} could not be read: {body}")]
    Http {
        host: String,
        status: Option<u16>,
        body: String,
    },
}

/// How much of a provider's error body travels to the UI. A failing gateway
/// answers with an HTML page, and the whole of one in a `<pre>` is not
/// evidence, it is a wall.
const BODY_LIMIT: usize = 600;

impl MrListError {
    /// A rejection the provider actually issued. `status` is the one it
    /// refused with, so a 403 never arrives claiming to be a 401.
    pub fn unauthorized(target: ListTarget<'_>, status: u16) -> Self {
        Self::Unauthorized {
            provider: target.kind.to_string(),
            host: target.host.to_string(),
            status,
        }
    }

    /// A failure that never reached the provider: the keyring answered `detail`
    /// instead of a token. Capped like a provider body — a keyring backend is
    /// as free to answer with a wall of text as a gateway is.
    pub fn no_credential(target: ListTarget<'_>, detail: impl Into<String>) -> Self {
        Self::NoCredential {
            provider: target.kind.to_string(),
            host: target.host.to_string(),
            detail: cap(&detail.into()),
        }
    }

    /// An internal failure with no provider status behind it. Named `other`
    /// rather than spelled at each call site so the "no status" case cannot be
    /// mistaken for a status the provider actually sent.
    pub fn other(host: &str, message: impl Into<String>) -> Self {
        Self::Http {
            host: host.to_string(),
            status: None,
            body: message.into(),
        }
    }
}

/// Decide what a provider's answer to a list request means.
///
/// `Ok(())` only for a 2xx — the whole point of the module.
///
/// The one genuinely ambiguous status is 403. GitHub spends it on both a token
/// without the `repo` scope and a secondary rate limit, and the body does not
/// reliably distinguish them, so the rate-limit headers decide: a
/// `Retry-After`, or an exhausted `X-RateLimit-Remaining`, means the token is
/// fine and the quota is not. Absent both, a 403 is a permission answer.
pub fn classify_list_response(
    target: ListTarget<'_>,
    response: ListResponse<'_>,
) -> Result<(), MrListError> {
    if response.status < 300 {
        return Ok(());
    }

    let retry_after = retry_after_seconds(response.headers);
    let throttled = retry_after.is_some() || quota_exhausted(response.headers);

    Err(match response.status {
        429 => MrListError::RateLimited {
            host: target.host.to_string(),
            retry_after,
        },
        403 if throttled => MrListError::RateLimited {
            host: target.host.to_string(),
            retry_after,
        },
        401 | 403 => MrListError::unauthorized(target, response.status),
        status => MrListError::Http {
            host: target.host.to_string(),
            status: Some(status),
            body: cap(response.body),
        },
    })
}

/// `Retry-After` in seconds, when the provider sent it that way.
///
/// RFC 9110 allows an HTTP-date instead, and both providers send the integer
/// form. Parsing a date needs a calendar and a clock — a clock in `domain/`
/// makes this function untestable without one — so the date form yields
/// `None`, and the UI degrades from "try again in 42s" to "shortly". That is a
/// smaller loss than the dependency.
fn retry_after_seconds(headers: &[(String, String)]) -> Option<u64> {
    header(headers, "retry-after")?.trim().parse().ok()
}

/// GitHub's secondary rate limit answers 403 with the remaining quota at zero.
fn quota_exhausted(headers: &[(String, String)]) -> bool {
    header(headers, "x-ratelimit-remaining").is_some_and(|v| v.trim() == "0")
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn cap(body: &str) -> String {
    let trimmed = body.trim();
    match trimmed.char_indices().nth(BODY_LIMIT) {
        Some((end, _)) => format!("{}…", &trimmed[..end]),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/mr_list_error.rs"]
mod tests;
