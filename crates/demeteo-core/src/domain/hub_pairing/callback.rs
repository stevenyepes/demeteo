//! What the loopback listener does with each request it receives while a
//! pairing waits for the Hub's redirect.
//!
//! Only a request that ends the wait is [`CallbackVerdict::Code`] or
//! [`CallbackVerdict::Denied`]; anything else is refused and the listener
//! keeps waiting. The `state` is checked before the outcome is read, so a
//! forged `error=access_denied` cannot cancel a pairing either.

use reqwest::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackVerdict {
    Code(String),
    Denied,
    Refuse(RefuseReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefuseReason {
    WrongPath,
    /// Missing, or not the one this pairing sent.
    WrongState,
    /// The right `state`, but no `code` to redeem and no `access_denied`.
    MissingCode,
}

impl RefuseReason {
    pub fn http_status(self) -> u16 {
        match self {
            Self::WrongPath => 404,
            Self::WrongState | Self::MissingCode => 400,
        }
    }
}

/// `path` and `query` are the request-target as received; `expected_path` is
/// the segment given to [`super::redirect_uri`], without its leading `/`.
///
/// `state` is compared with plain equality: it is single-use, and only a
/// process on this machine can reach the loopback listener to time it.
pub fn callback_verdict(
    path: &str,
    query: Option<&str>,
    expected_path: &str,
    expected_state: &str,
) -> CallbackVerdict {
    if path.strip_prefix('/') != Some(expected_path) {
        return CallbackVerdict::Refuse(RefuseReason::WrongPath);
    }
    let pairs: Vec<(String, String)> = query
        .and_then(|query| Url::parse(&format!("http://127.0.0.1/?{query}")).ok())
        .map(|url| url.query_pairs().into_owned().collect())
        .unwrap_or_default();
    let param = |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    if param("state").as_deref() != Some(expected_state) {
        return CallbackVerdict::Refuse(RefuseReason::WrongState);
    }
    if let Some(error) = param("error") {
        return match error.as_str() {
            "access_denied" => CallbackVerdict::Denied,
            _ => CallbackVerdict::Refuse(RefuseReason::MissingCode),
        };
    }
    match param("code") {
        Some(code) if !code.is_empty() => CallbackVerdict::Code(code),
        _ => CallbackVerdict::Refuse(RefuseReason::MissingCode),
    }
}
