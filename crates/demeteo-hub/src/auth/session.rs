//! What a sign-in leaves in the browser, and the check every state-changing
//! request passes before anything reads the database: synchronous, no I/O
//! beyond the CSPRNG.
//!
//! One secret has three forms, and only the first can be turned into the
//! others. The cookie carries the secret itself and is `HttpOnly`. The store
//! keeps `SHA-256(secret)`. The CSRF token is a digest of the secret under its
//! own label, so page script holds a value that is neither the cookie nor the
//! stored hash, and no column is needed to check it.
//!
//! Neither the secret nor the token may reach a `tracing` call, a `Debug`
//! output or an error text. [`SessionSecret`] derives nothing for that reason.

use super::challenge::{base64url_decode, base64url_encode, RandomSourceFailed};
use crate::store::sessions::SessionHash;
use sha2::{Digest, Sha256};

/// The `__Host-` prefix makes the browser refuse the cookie unless it is
/// `Secure`, has `Path=/` and has no `Domain`, so a sibling subdomain can
/// neither read this cookie nor plant one of this name.
pub const COOKIE_NAME: &str = "__Host-demeteo_hub_session";

pub const CSRF_HEADER: &str = "x-csrf-token";

pub const SESSION_TTL_SECS: u64 = 43_200;

const CSRF_LABEL: &[u8] = b"demeteo-hub/v1/csrf";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    RandomSource(#[from] RandomSourceFailed),
}

pub struct SessionSecret([u8; 32]);

impl SessionSecret {
    pub fn generate() -> Result<Self, SessionError> {
        let mut secret = [0u8; 32];
        aws_lc_rs::rand::fill(&mut secret).map_err(|_| RandomSourceFailed)?;
        Ok(Self(secret))
    }

    /// The secret in the value of a `Cookie` request header, or `None` when
    /// [`COOKIE_NAME`] is not among its pairs or does not hold 32 bytes.
    pub fn from_cookie_header(header: &str) -> Option<Self> {
        let value = header.split(';').find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == COOKIE_NAME).then_some(value)
        })?;
        <[u8; 32]>::try_from(base64url_decode(value)?)
            .ok()
            .map(Self)
    }

    pub fn cookie_value(&self) -> String {
        base64url_encode(&self.0)
    }

    pub fn csrf_token(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(CSRF_LABEL);
        hasher.update([0u8]);
        hasher.update(self.0);
        base64url_encode(&hasher.finalize())
    }

    /// The form the store keeps. It is taken here because the bytes it is a
    /// digest of have no accessor.
    pub fn hash(&self) -> SessionHash {
        SessionHash::digest(&self.0)
    }
}

impl std::fmt::Debug for SessionSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionSecret(<redacted>)")
    }
}

/// The `Set-Cookie` value that starts a session.
///
/// `Secure` is unconditional. Caddy terminates TLS, so the scheme of the
/// request the Hub sees says nothing about the one the browser used, and
/// inferring the attribute from it would drop it in production. Neither it nor
/// `SameSite=Strict` is relaxed for development.
pub fn set_cookie(secret: &SessionSecret) -> String {
    cookie(&secret.cookie_value(), SESSION_TTL_SECS)
}

/// The `Set-Cookie` value that ends a session. A browser removes a cookie only
/// when the name and the path match the ones it was set with.
pub fn clear_cookie() -> String {
    cookie("", 0)
}

fn cookie(value: &str, max_age: u64) -> String {
    format!("{COOKIE_NAME}={value}; Path=/; Max-Age={max_age}; HttpOnly; Secure; SameSite=Strict")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("the request does not come from the Hub's own origin")]
    OriginMismatch,
    #[error("the CSRF token is missing")]
    CsrfMissing,
    #[error("the CSRF token does not belong to this session")]
    CsrfMismatch,
}

/// Whether a state-changing request may go on to its handler. `origin`,
/// `cookie` and `csrf` are the values of the request's `Origin`, `Cookie` and
/// [`CSRF_HEADER`] headers; `expected_origin` comes from the Hub's
/// configuration and never from the request.
///
/// A route with no session to derive a token from passes `needs_session:
/// false`, and the origin check is all that guards it.
///
/// A token arriving with no usable cookie is let through: that request is
/// unauthenticated, not forged, and the session lookup that follows answers
/// it. Refusing it here would report a missing sign-in as a CSRF failure.
pub fn guard_mutation(
    expected_origin: &str,
    origin: Option<&str>,
    cookie: Option<&str>,
    csrf: Option<&str>,
    needs_session: bool,
) -> Result<(), Rejection> {
    if origin != Some(expected_origin) {
        return Err(Rejection::OriginMismatch);
    }
    if !needs_session {
        return Ok(());
    }
    let presented = csrf
        .filter(|token| !token.is_empty())
        .ok_or(Rejection::CsrfMissing)?;
    match cookie.and_then(SessionSecret::from_cookie_header) {
        Some(secret) if !same_digest(presented, &secret.csrf_token()) => {
            Err(Rejection::CsrfMismatch)
        }
        _ => Ok(()),
    }
}

/// Equality of the digests, not of the strings: how long a string comparison
/// runs tells the caller how much of a guessed token was right.
fn same_digest(presented: &str, expected: &str) -> bool {
    Sha256::digest(presented.as_bytes()) == Sha256::digest(expected.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://hub.example";

    /// The secret is the bytes `01..=20` (hex). `COOKIE_VALUE` is its
    /// base64url, and `CSRF_TOKEN` was computed outside this crate, with
    /// Python's `hashlib`, over `b"demeteo-hub/v1/csrf" + b"\x00" + secret`.
    const COOKIE_VALUE: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";
    const CSRF_TOKEN: &str = "QlGQJgsNx1XQwOVU1RiYHGnxWx3OmpUgZ43kWyjl2t8";
    /// `SHA-256(secret)`, the form the store keeps.
    const SESSION_HASH: &str = "riFsLvUkejeCwTXvonmj5M3GEJQnD10r5YxiBLemEsk";

    fn secret() -> SessionSecret {
        SessionSecret(std::array::from_fn(|i| 1 + i as u8))
    }

    fn cookie_header() -> String {
        format!("{COOKIE_NAME}={COOKIE_VALUE}")
    }

    #[test]
    fn the_cookie_value_is_base64url_of_the_secret() {
        assert_eq!(secret().cookie_value(), COOKIE_VALUE);
    }

    #[test]
    fn the_csrf_token_is_the_labelled_digest_of_the_secret() {
        assert_eq!(secret().csrf_token(), CSRF_TOKEN);
    }

    #[test]
    fn the_csrf_token_is_stable_and_is_neither_the_cookie_nor_the_stored_hash() {
        let secret = secret();
        assert_eq!(secret.csrf_token(), secret.csrf_token());
        assert_ne!(secret.csrf_token(), secret.cookie_value());
        assert_eq!(base64url_encode(&Sha256::digest(secret.0)), SESSION_HASH);
        assert_ne!(secret.csrf_token(), SESSION_HASH);
    }

    #[test]
    fn the_stored_hash_is_over_the_secret_and_not_its_cookie_text() {
        let bytes = base64url_decode(COOKIE_VALUE).unwrap();
        assert_eq!(secret().hash(), SessionHash::digest(&bytes));
        assert_ne!(
            secret().hash(),
            SessionHash::digest(COOKIE_VALUE.as_bytes())
        );
    }

    #[test]
    fn two_secrets_have_different_csrf_tokens() {
        assert_ne!(secret().csrf_token(), SessionSecret([0u8; 32]).csrf_token());
    }

    #[test]
    fn a_generated_secret_is_32_fresh_bytes() {
        let first = SessionSecret::generate().unwrap();
        let second = SessionSecret::generate().unwrap();
        assert_ne!(first.0, [0u8; 32]);
        assert_ne!(first.0, second.0);
        assert_eq!(
            base64url_decode(&first.cookie_value()),
            Some(first.0.to_vec())
        );
    }

    #[test]
    fn debug_shows_neither_the_secret_nor_what_is_derived_from_it() {
        let secret = secret();
        let shown = format!("{secret:?} {:?}", Some(&secret));
        assert!(!shown.contains(COOKIE_VALUE), "{shown}");
        assert!(!shown.contains(CSRF_TOKEN), "{shown}");
        assert!(!shown.contains("1, 2, 3"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn the_secret_is_found_among_several_cookies() {
        for header in [
            cookie_header(),
            format!("theme=dark; {}; lang=en", cookie_header()),
            format!("theme=dark;{}", cookie_header()),
            format!("a=b=c; novalue; {}", cookie_header()),
        ] {
            let found = SessionSecret::from_cookie_header(&header);
            assert_eq!(
                found.map(|secret| secret.0),
                Some(secret().0),
                "in `{header}`"
            );
        }
    }

    #[test]
    fn a_cookie_header_without_a_32_byte_session_value_yields_no_secret() {
        let short = base64url_encode(&[7u8; 31]);
        let long = base64url_encode(&[7u8; 33]);
        for header in [
            String::new(),
            "theme=dark".to_owned(),
            format!("{COOKIE_NAME}="),
            format!("{COOKIE_NAME}={short}"),
            format!("{COOKIE_NAME}={long}"),
            format!("{COOKIE_NAME}={COOKIE_VALUE}="),
            format!("{COOKIE_NAME}=not base64url"),
            format!("x{COOKIE_NAME}={COOKIE_VALUE}"),
            format!("{COOKIE_NAME}x={COOKIE_VALUE}"),
            format!("theme={COOKIE_VALUE}"),
        ] {
            assert!(
                SessionSecret::from_cookie_header(&header).is_none(),
                "from `{header}`"
            );
        }
    }

    #[test]
    fn set_cookie_is_exactly_the_session_cookie() {
        assert_eq!(
            set_cookie(&secret()),
            format!(
                "__Host-demeteo_hub_session={COOKIE_VALUE}; Path=/; Max-Age=43200; \
                 HttpOnly; Secure; SameSite=Strict"
            )
        );
    }

    #[test]
    fn clear_cookie_empties_the_same_cookie_and_expires_it() {
        assert_eq!(
            clear_cookie(),
            "__Host-demeteo_hub_session=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict"
        );
    }

    #[test]
    fn the_cookie_set_at_sign_in_is_the_one_read_back() {
        let secret = SessionSecret::generate().unwrap();
        let set = set_cookie(&secret);
        let pair = set.split(';').next().unwrap();
        let read = SessionSecret::from_cookie_header(pair);
        assert_eq!(read.map(|read| read.0), Some(secret.0));
    }

    type Case = (
        &'static str,
        Option<&'static str>,
        Option<String>,
        Option<String>,
        bool,
        Result<(), Rejection>,
    );

    #[test]
    fn guard_mutation_decides_origin_first_then_the_token_then_its_match() {
        let cookie = Some(cookie_header());
        let token = Some(CSRF_TOKEN.to_owned());
        let other_token = Some(SessionSecret([0u8; 32]).csrf_token());
        let origin = Some(ORIGIN);
        let cases: [Case; 16] = [
            (
                "no origin, pre-session",
                None,
                None,
                None,
                false,
                Err(Rejection::OriginMismatch),
            ),
            (
                "foreign origin, pre-session",
                Some("https://evil.example"),
                None,
                None,
                false,
                Err(Rejection::OriginMismatch),
            ),
            (
                "no origin, otherwise valid",
                None,
                cookie.clone(),
                token.clone(),
                true,
                Err(Rejection::OriginMismatch),
            ),
            (
                "foreign origin, otherwise valid",
                Some("https://evil.example"),
                cookie.clone(),
                token.clone(),
                true,
                Err(Rejection::OriginMismatch),
            ),
            (
                "origin with a trailing slash",
                Some("https://hub.example/"),
                cookie.clone(),
                token.clone(),
                true,
                Err(Rejection::OriginMismatch),
            ),
            (
                "the literal null origin",
                Some("null"),
                cookie.clone(),
                token.clone(),
                true,
                Err(Rejection::OriginMismatch),
            ),
            ("own origin, pre-session", origin, None, None, false, Ok(())),
            (
                "pre-session ignores a wrong token",
                origin,
                cookie.clone(),
                other_token.clone(),
                false,
                Ok(()),
            ),
            (
                "cookie, no token",
                origin,
                cookie.clone(),
                None,
                true,
                Err(Rejection::CsrfMissing),
            ),
            (
                "cookie, empty token",
                origin,
                cookie.clone(),
                Some(String::new()),
                true,
                Err(Rejection::CsrfMissing),
            ),
            (
                "no cookie, no token",
                origin,
                None,
                None,
                true,
                Err(Rejection::CsrfMissing),
            ),
            (
                "another session's token",
                origin,
                cookie.clone(),
                other_token,
                true,
                Err(Rejection::CsrfMismatch),
            ),
            (
                "the cookie value as the token",
                origin,
                cookie.clone(),
                Some(COOKIE_VALUE.to_owned()),
                true,
                Err(Rejection::CsrfMismatch),
            ),
            (
                "cookie and its token",
                origin,
                cookie.clone(),
                token.clone(),
                true,
                Ok(()),
            ),
            (
                "token, no cookie",
                origin,
                None,
                token.clone(),
                true,
                Ok(()),
            ),
            (
                "token, unusable cookie",
                origin,
                Some(format!("{COOKIE_NAME}=AAAA")),
                token,
                true,
                Ok(()),
            ),
        ];
        for (name, origin, cookie, csrf, needs_session, expected) in cases {
            assert_eq!(
                guard_mutation(
                    ORIGIN,
                    origin,
                    cookie.as_deref(),
                    csrf.as_deref(),
                    needs_session
                ),
                expected,
                "{name}"
            );
        }
    }
}
