//! The Hub's whole configuration surface, read from the environment.
//!
//! This is the only file allowed to spell a host, port or filesystem root
//! (`tests/no_literals.rs`), so an image never bakes one in.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_WEB_DIR: &str = "/srv/hub-web";
const DEFAULT_TRANSIT_MOUNT: &str = "transit";
const DEFAULT_TRANSIT_KEY: &str = "demeteo-hub";

const DATABASE_URL: &str = "DATABASE_URL";
const BIND: &str = "DEMETEO_HUB_BIND";
const PUBLIC_URL: &str = "DEMETEO_HUB_PUBLIC_URL";
const RP_ID: &str = "DEMETEO_HUB_RP_ID";
const WEB_DIR: &str = "DEMETEO_HUB_WEB_DIR";
const OPENBAO_ADDR: &str = "OPENBAO_ADDR";
const OPENBAO_TOKEN: &str = "OPENBAO_TOKEN";
const OPENBAO_TOKEN_FILE: &str = "OPENBAO_TOKEN_FILE";
const TRANSIT_MOUNT: &str = "DEMETEO_HUB_TRANSIT_MOUNT";
const TRANSIT_KEY: &str = "DEMETEO_HUB_TRANSIT_KEY";

const REDACTED: &str = "<redacted>";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("required environment variable `{0}` is not set")]
    Missing(&'static str),
    #[error("`{var}` is invalid: {reason}")]
    Invalid { var: &'static str, reason: String },
    #[error("cannot read the file named by `{var}`: {reason}")]
    TokenFile { var: &'static str, reason: String },
}

pub struct Config {
    pub bind: SocketAddr,
    /// The single authority for URLs and WebAuthn origins; never derived from
    /// a request's `Host` header.
    pub public_url: String,
    pub rp_id: String,
    pub database_url: String,
    pub openbao_addr: String,
    pub openbao_token: String,
    pub transit_mount: String,
    pub transit_key: String,
    pub web_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let required = |var: &'static str| {
            lookup(var)
                .filter(|v| !v.is_empty())
                .ok_or(ConfigError::Missing(var))
        };
        let or_default = |var: &'static str, default: &str| {
            lookup(var)
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_owned())
        };

        let bind_text = or_default(BIND, DEFAULT_BIND);
        let bind = bind_text.parse().map_err(|e| ConfigError::Invalid {
            var: BIND,
            reason: format!("`{bind_text}` is not a socket address: {e}"),
        })?;

        let public_url = required(PUBLIC_URL)?;
        if !public_url_scheme_allowed(&public_url) {
            return Err(ConfigError::Invalid {
                var: PUBLIC_URL,
                reason: format!(
                    "`{public_url}` must be an `https://` URL: WebAuthn needs HTTPS, and \
                     `http://localhost` is the only exception"
                ),
            });
        }
        let rp_id = required(RP_ID)?.to_ascii_lowercase();
        if !rp_id_covers(&rp_id, &public_url) {
            return Err(ConfigError::Invalid {
                var: RP_ID,
                reason: format!(
                    "`{rp_id}` must be `localhost` or a dotted domain equal to, or a domain \
                     suffix of, the host of `{PUBLIC_URL}`"
                ),
            });
        }

        Ok(Self {
            bind,
            public_url,
            rp_id,
            database_url: required(DATABASE_URL)?,
            openbao_addr: required(OPENBAO_ADDR)?,
            openbao_token: openbao_token(&lookup)?,
            transit_mount: or_default(TRANSIT_MOUNT, DEFAULT_TRANSIT_MOUNT),
            transit_key: or_default(TRANSIT_KEY, DEFAULT_TRANSIT_KEY),
            web_dir: PathBuf::from(or_default(WEB_DIR, DEFAULT_WEB_DIR)),
        })
    }
}

fn openbao_token(lookup: &impl Fn(&str) -> Option<String>) -> Result<String, ConfigError> {
    if let Some(path) = lookup(OPENBAO_TOKEN_FILE).filter(|p| !p.is_empty()) {
        let text = std::fs::read_to_string(&path).map_err(|e| ConfigError::TokenFile {
            var: OPENBAO_TOKEN_FILE,
            reason: e.to_string(),
        })?;
        let token = text.trim();
        if token.is_empty() {
            return Err(ConfigError::Invalid {
                var: OPENBAO_TOKEN_FILE,
                reason: "the file is empty".to_owned(),
            });
        }
        return Ok(token.to_owned());
    }
    lookup(OPENBAO_TOKEN)
        .filter(|v| !v.is_empty())
        .ok_or(ConfigError::Missing(OPENBAO_TOKEN))
}

fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host_port.starts_with('[') {
        return host_port
            .find(']')
            .map_or(host_port, |end| &host_port[..=end]);
    }
    host_port.split(':').next().unwrap_or(host_port)
}

/// WebAuthn refuses a non-secure origin at enrolment, and browsers treat
/// `http://localhost` as the one secure plain-HTTP origin, so anything else
/// is refused at startup rather than at the first ceremony.
fn public_url_scheme_allowed(public_url: &str) -> bool {
    let Some((scheme, _)) = public_url.split_once("://") else {
        return false;
    };
    scheme.eq_ignore_ascii_case("https")
        || (scheme.eq_ignore_ascii_case("http")
            && url_host(public_url).eq_ignore_ascii_case("localhost"))
}

/// Browsers lowercase the host before matching, so both sides are compared
/// lowercased. A single-label RP ID other than `localhost` is refused here
/// because the browser would refuse it at enrolment; a full public-suffix
/// check needs the PSL, which this crate does not carry.
fn rp_id_covers(rp_id: &str, public_url: &str) -> bool {
    let rp_id = rp_id.to_ascii_lowercase();
    let host = url_host(public_url).to_ascii_lowercase();
    if rp_id != "localhost" && !rp_id.contains('.') {
        return false;
    }
    host == rp_id
        || host
            .strip_suffix(&rp_id)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// `scheme://user:password@host/db?password=x` →
/// `scheme://user:<redacted>@host/db?password=<redacted>`.
///
/// Fails closed: a password holding `#`, `?` or `/` moves its `@` past the
/// authority, so any `@` after the authority means the shape was not
/// understood and the whole URL is withheld rather than echoed. `main.rs`
/// logs `?config` on every start, so a verbatim fallback here is a password
/// in the container log.
fn redact_database_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return REDACTED.to_owned();
    };
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return REDACTED.to_owned();
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    if tail.contains('@') {
        return REDACTED.to_owned();
    }
    let authority = match authority.rsplit_once('@') {
        Some((userinfo, host)) => {
            let user = userinfo.split(':').next().unwrap_or(userinfo);
            format!("{user}:{REDACTED}@{host}")
        }
        None => authority.to_owned(),
    };
    let (before_fragment, fragment) = tail
        .split_once('#')
        .map_or((tail, None), |(b, f)| (b, Some(f)));
    let (path, query) = before_fragment
        .split_once('?')
        .map_or((before_fragment, None), |(p, q)| (p, Some(q)));
    let mut out = format!("{scheme}://{authority}{path}");
    if let Some(query) = query {
        out.push('?');
        out.push_str(&redact_query(query));
    }
    if fragment.is_some() {
        out.push('#');
        out.push_str(REDACTED);
    }
    out
}

/// sqlx percent-decodes query keys, so a key holding `%` could spell
/// `password` and is redacted as if it did.
fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, _))
                if key.contains('%') || key.to_ascii_lowercase().contains("password") =>
            {
                format!("{key}={REDACTED}")
            }
            _ => pair.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("bind", &self.bind)
            .field("public_url", &self.public_url)
            .field("rp_id", &self.rp_id)
            .field("database_url", &redact_database_url(&self.database_url))
            .field("openbao_addr", &self.openbao_addr)
            .field("openbao_token", &REDACTED)
            .field("transit_mount", &self.transit_mount)
            .field("transit_key", &self.transit_key)
            .field("web_dir", &self.web_dir)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn full() -> HashMap<&'static str, String> {
        HashMap::from([
            (
                DATABASE_URL,
                "postgres://hub:s3cretpw@db:5432/hub".to_owned(),
            ),
            (PUBLIC_URL, "https://hub.example.com".to_owned()),
            (RP_ID, "example.com".to_owned()),
            (OPENBAO_ADDR, "http://bao:8200".to_owned()),
            (OPENBAO_TOKEN, "s.tokensecret".to_owned()),
        ])
    }

    fn load(env: &HashMap<&'static str, String>) -> Result<Config, ConfigError> {
        Config::from_lookup(|name| env.get(name).cloned())
    }

    #[test]
    fn complete_environment_yields_a_config_with_defaults() {
        let config = load(&full()).unwrap();
        assert_eq!(config.bind.to_string(), DEFAULT_BIND);
        assert_eq!(config.transit_mount, DEFAULT_TRANSIT_MOUNT);
        assert_eq!(config.transit_key, DEFAULT_TRANSIT_KEY);
    }

    #[test]
    fn each_missing_required_variable_is_named_in_the_error() {
        for var in [DATABASE_URL, PUBLIC_URL, RP_ID, OPENBAO_ADDR, OPENBAO_TOKEN] {
            let mut env = full();
            env.remove(var);
            let err = load(&env).unwrap_err().to_string();
            assert!(err.contains(var), "{var}: {err}");
        }
    }

    #[test]
    fn token_file_wins_and_is_trimmed() {
        let path = std::env::temp_dir().join(format!("hub-token-{}", std::process::id()));
        std::fs::write(&path, "s.fromfile\n").unwrap();
        let mut env = full();
        env.remove(OPENBAO_TOKEN);
        env.insert(OPENBAO_TOKEN_FILE, path.to_string_lossy().into_owned());
        let config = load(&env);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(config.unwrap().openbao_token, "s.fromfile");
    }

    #[test]
    fn unreadable_token_file_is_an_error_naming_the_variable() {
        let mut env = full();
        env.insert(OPENBAO_TOKEN_FILE, "no-such-token-file".to_owned());
        let err = load(&env).unwrap_err().to_string();
        assert!(err.contains(OPENBAO_TOKEN_FILE), "{err}");
    }

    #[test]
    fn debug_hides_the_database_password_and_the_token() {
        let shown = format!("{:?}", load(&full()).unwrap());
        assert!(!shown.contains("s3cretpw"), "{shown}");
        assert!(!shown.contains("s.tokensecret"), "{shown}");
        assert!(shown.contains("postgres://hub:"), "{shown}");
    }

    #[test]
    fn rp_id_must_be_a_domain_suffix_of_the_public_host() {
        for (url, rp, ok) in [
            ("https://hub.example.com", "example.com", true),
            ("https://hub.example.com:8443/x", "hub.example.com", true),
            ("https://evilexample.com", "example.com", false),
            ("https://hub.example.com", "other.org", false),
        ] {
            assert_eq!(rp_id_covers(rp, url), ok, "{url} / {rp}");
        }
        let mut env = full();
        env.insert(RP_ID, "other.org".to_owned());
        assert!(load(&env).unwrap_err().to_string().contains(RP_ID));
    }

    #[test]
    fn rp_id_is_compared_case_insensitively_and_stored_lowercased() {
        assert!(rp_id_covers("hub.example.com", "https://Hub.Example.com"));
        assert!(rp_id_covers("Example.COM", "https://hub.example.com"));
        let mut env = full();
        env.insert(PUBLIC_URL, "https://Hub.Example.com".to_owned());
        env.insert(RP_ID, "Hub.Example.com".to_owned());
        assert_eq!(load(&env).unwrap().rp_id, "hub.example.com");
    }

    #[test]
    fn a_single_label_rp_id_other_than_localhost_is_rejected() {
        assert!(!rp_id_covers("com", "https://hub.com"));
        assert!(!rp_id_covers("com", "https://com"));
        assert!(rp_id_covers("localhost", "http://localhost:8080"));
        assert!(rp_id_covers("LocalHost", "http://localhost"));
        let mut env = full();
        env.insert(PUBLIC_URL, "https://hub.com".to_owned());
        env.insert(RP_ID, "com".to_owned());
        assert!(load(&env).unwrap_err().to_string().contains(RP_ID));
    }

    #[test]
    fn public_url_must_be_https_unless_its_host_is_localhost() {
        for (url, ok) in [
            ("http://hub.example.com", false),
            ("https://hub.example.com", true),
            ("http://localhost:8080", true),
            ("HTTPS://Hub.Example.com", true),
            ("ftp://hub.example.com", false),
            ("hub.example.com", false),
        ] {
            assert_eq!(public_url_scheme_allowed(url), ok, "{url}");
        }
        let mut env = full();
        env.insert(PUBLIC_URL, "http://hub.example.com".to_owned());
        let err = load(&env).unwrap_err().to_string();
        assert!(err.contains(PUBLIC_URL) && err.contains("HTTPS"), "{err}");
    }

    #[test]
    fn url_host_keeps_a_bracketed_ipv6_literal_whole() {
        assert_eq!(url_host("http://[::1]:8080"), "[::1]");
        assert_eq!(url_host("https://user@[2001:db8::1]/x"), "[2001:db8::1]");
        assert!(!rp_id_covers("localhost", "http://[::1]:8080"));
    }

    fn assert_token_file_rejected(content: &str, tag: &str) {
        let path = std::env::temp_dir().join(format!("hub-token-{tag}-{}", std::process::id()));
        std::fs::write(&path, content).unwrap();
        let mut env = full();
        env.remove(OPENBAO_TOKEN);
        env.insert(OPENBAO_TOKEN_FILE, path.to_string_lossy().into_owned());
        let config = load(&env);
        std::fs::remove_file(&path).unwrap();
        let err = config.unwrap_err().to_string();
        assert!(err.contains(OPENBAO_TOKEN_FILE), "{err}");
    }

    #[test]
    fn an_empty_token_file_is_an_error_naming_the_variable() {
        assert_token_file_rejected("", "empty");
    }

    #[test]
    fn a_whitespace_only_token_file_is_an_error_naming_the_variable() {
        assert_token_file_rejected(" \n\t\n", "blank");
    }

    /// Every URL here hides `s3cret` somewhere the authority-only parse
    /// cannot see: a reserved character in the password moves the `@` past
    /// the authority, or the secret travels in the query or the fragment.
    #[test]
    fn redaction_never_shows_a_password_in_an_unusual_url_shape() {
        for url in [
            "postgres://demeteo:pa#s3cret@postgres:5432/db",
            "postgres://demeteo:pa?s3cret@postgres/db",
            "postgres://demeteo:pa/s3cret@postgres/db",
            "postgres://demeteo:a@b/s3cret@postgres/db",
            "postgres://hub@db/hub?password=s3cret",
            "postgres://hub@db/hub?sslmode=disable&password=s3cret",
            "postgres://hub@db/hub?sslmode=disable&SSLPassword=s3cret",
            "postgres://hub@db/hub?pass%77ord=s3cret",
            "postgres://hub@db/hub?password=s3cret#top",
            "postgres://db/hub#s3cret",
        ] {
            let redacted = redact_database_url(url);
            assert!(!redacted.contains("s3cret"), "{url} -> {redacted}");
            let mut env = full();
            env.insert(DATABASE_URL, url.to_owned());
            let shown = format!("{:?}", load(&env).unwrap());
            assert!(!shown.contains("s3cret"), "{url} -> {shown}");
        }
    }

    #[test]
    fn an_unrecognised_url_shape_redacts_entirely() {
        assert_eq!(
            redact_database_url("postgres://demeteo:pa#ss@postgres:5432/db"),
            REDACTED
        );
        assert_eq!(redact_database_url("not a url"), REDACTED);
    }

    #[test]
    fn redaction_keeps_the_non_secret_parts_of_a_recognised_url() {
        assert_eq!(
            redact_database_url("postgres://hub:pw@db:5432/hub?sslmode=disable&password=pw"),
            "postgres://hub:<redacted>@db:5432/hub?sslmode=disable&password=<redacted>"
        );
    }

    #[test]
    fn redaction_leaves_a_url_without_credentials_alone() {
        assert_eq!(
            redact_database_url("postgres://db/hub"),
            "postgres://db/hub"
        );
    }
}
