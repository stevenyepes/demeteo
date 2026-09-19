//! What `POST /register` will store. Registration is unauthenticated by design
//! (`docs/MCP_INTEGRATION.md` §9), so the only things standing between an
//! arbitrary local caller and the consent screen are these bounds and the human
//! reading that screen — which is why every field the screen renders, or
//! redirects to, is limited here rather than trusted.

use thiserror::Error;

pub const MAX_CLIENT_NAME_CHARS: usize = 100;
pub const MAX_REDIRECT_URIS: usize = 5;
pub const MAX_REDIRECT_URI_CHARS: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistrationError {
    #[error("client_name must be 1-{MAX_CLIENT_NAME_CHARS} characters with no control characters")]
    ClientName,
    #[error("redirect_uris must hold 1-{MAX_REDIRECT_URIS} entries")]
    RedirectUriCount,
    #[error("redirect_uri {0:?} is not an https URL, a loopback http URL, or a private-use scheme, or carries a fragment or credentials")]
    RedirectUri(String),
}

/// RFC 8252's three native-app redirect shapes, and nothing else: a claimed
/// `https` URL, `http` to a loopback literal (the port is the client's to
/// choose), or a private-use scheme in reverse-domain form (it must contain a
/// `.`, which also keeps `javascript:`, `data:`, `file:` and friends out).
/// A fragment is refused because the authorization response is a query string;
/// userinfo is refused because it makes the displayed host lie.
pub fn validate_registration(
    client_name: &str,
    redirect_uris: &[String],
) -> Result<(), RegistrationError> {
    let name = client_name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_CLIENT_NAME_CHARS
        || name.chars().any(is_control_or_invisible)
    {
        return Err(RegistrationError::ClientName);
    }
    if redirect_uris.is_empty() || redirect_uris.len() > MAX_REDIRECT_URIS {
        return Err(RegistrationError::RedirectUriCount);
    }
    for uri in redirect_uris {
        if !redirect_uri_is_acceptable(uri) {
            return Err(RegistrationError::RedirectUri(uri.clone()));
        }
    }
    Ok(())
}

/// Bidi overrides and zero-width characters are not `char::is_control`, and
/// they are how a name on the consent screen is made to read as another one.
fn is_control_or_invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
        )
}

fn redirect_uri_is_acceptable(uri: &str) -> bool {
    if uri.is_empty()
        || uri.len() > MAX_REDIRECT_URI_CHARS
        || uri.contains('#')
        || uri.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    let scheme_ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok {
        return false;
    }
    match scheme.to_ascii_lowercase().as_str() {
        "https" => authority(rest).is_some_and(|host| !host.is_empty()),
        "http" => authority(rest).is_some_and(is_loopback_host),
        other => other.contains('.'),
    }
}

/// The host of a `//authority/...` remainder, without port; `None` when there
/// is no authority or it carries userinfo.
fn authority(rest: &str) -> Option<&str> {
    let after = rest.strip_prefix("//")?;
    let end = after.find(['/', '?']).unwrap_or(after.len());
    let authority = &after[..end];
    if authority.contains('@') {
        return None;
    }
    if let Some(bracketed) = authority.strip_prefix('[') {
        let close = bracketed.find(']')?;
        return Some(&authority[..close + 2]);
    }
    Some(authority.split(':').next().unwrap_or(authority))
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "[::1]" | "localhost")
}

#[cfg(test)]
#[path = "../../../tests/domain/oauth/registration.rs"]
mod tests;
