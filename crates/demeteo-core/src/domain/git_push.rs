//! What a push to `origin` needs before it runs, and how to read one that
//! failed. See [`crate::domain`].
//!
//! Both questions are answered from text — a remote URL, git's stderr — and
//! neither needs a port, which is the whole reason they live here rather than
//! inside the `async fn` that pushes. [`unattended_env`] is the third: it is
//! the push's answer to "nobody is here to type a password", and every other
//! `git` Demeteo runs turned out to need the same one.

/// The host an HTTPS remote must authenticate against, or `None` when the push
/// carries its own credential and Demeteo must not supply one.
///
/// The `None` cases are not a fallback, they are the answer:
///
/// * **ssh / `git@host:path`** — the user's own key already authenticates it,
///   and an inline credential helper installed over it would do nothing.
/// * **a URL that already carries a password** (`https://user:tok@host/…`) —
///   something deliberately put a secret there; overriding it would silently
///   authenticate as somebody else.
/// * **anything not http(s)** — `file://`, a bare path, a named remote.
///
/// The userinfo has to be skipped rather than parsed as the host, because the
/// form Demeteo itself writes is exactly that: `mr_publisher` rewrites `origin`
/// to `https://x-access-token@github.com/owner/repo` — deliberately *token
/// free*, so the PAT is never persisted in `.git/config` — and a reader that
/// took `x-access-token@github.com` for the host would match no configured
/// provider and conclude the push needed no credential. That conclusion is how
/// every non-MR push in the app came to fail on a project the MR publisher had
/// touched once.
pub fn credential_host(remote_url: &str) -> Option<&str> {
    let remote = HttpRemote::parse(remote_url)?;
    // A colon in the userinfo is a password already in hand.
    if remote.userinfo.is_some_and(|u| u.contains(':')) {
        return None;
    }
    let host = host_without_port(remote.host_port);
    (!host.is_empty()).then_some(host)
}

/// `host[:port]` as the bare host — the form git's credential protocol and
/// [`GitCredential`](crate::adapters::git_push::GitCredential) both name, so a
/// provider stored as `gitlab.local:8443` still matches what git sends.
pub fn host_without_port(host_port: &str) -> &str {
    host_port.split(':').next().unwrap_or(host_port)
}

/// The same remote with the password dropped from its userinfo, or `None` when
/// there is nothing to drop.
///
/// This is the inverse case of [`credential_host`]: that one declines a URL
/// that carries a password, this one is what turns such a URL into the
/// token-free form `credential_host` accepts — `scheme://<user>@host[:port]/…`,
/// path and query untouched; a password with no user (`https://:tok@host/r`)
/// leaves no userinfo at all rather than an empty `@host`. A password inline in `origin` sits in
/// `.git/config` in the clear, so the caller has to move it out before a
/// credential helper can supply it instead.
///
/// `None` is the answer for everything that is not an http(s) URL with a
/// password: ssh and `git@host:path` authenticate with the user's key, and a
/// URL already without a password has nothing to rewrite. The latter also makes
/// the function idempotent — its own output is never rewritten again.
pub fn token_free_origin(remote_url: &str) -> Option<String> {
    let remote = HttpRemote::parse(remote_url)?;
    let (user, _password) = remote.userinfo?.split_once(':')?;
    let userinfo = if user.is_empty() {
        String::new()
    } else {
        format!("{user}@")
    };
    Some(format!(
        "{}{userinfo}{}{}",
        remote.scheme, remote.host_port, remote.tail
    ))
}

/// The password embedded in an http(s) URL's userinfo, exactly as written —
/// still percent-encoded if the URL spelled it that way.
///
/// Shares `HttpRemote::parse` with [`token_free_origin`], so the two agree on
/// which URLs carry a password at all: this is `Some` precisely when that one
/// is. A password containing an unencoded `/`, `?` or `#` ends the authority
/// early and is therefore never seen — see `docs/KNOWN_ISSUES.md`.
///
/// Git decodes the userinfo before it sends it, so a caller comparing against a
/// stored secret has to try [`percent_decode`] of this value too.
pub fn embedded_password(remote_url: &str) -> Option<String> {
    let (_user, password) = HttpRemote::parse(remote_url)?.userinfo?.split_once(':')?;
    Some(password.to_string())
}

/// `%XX` escapes decoded, any other byte kept; `None` when the result is not
/// UTF-8. A malformed escape (`%zz`, a trailing `%`) is left literal, which is
/// what git does with it.
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Whether the password `remote_url` carries is `secret`, compared both as
/// written and percent-decoded.
pub fn embeds_secret(remote_url: &str, secret: &str) -> bool {
    let Some(raw) = embedded_password(remote_url) else {
        return false;
    };
    raw == secret || percent_decode(&raw).is_some_and(|decoded| decoded == secret)
}

struct HttpRemote<'a> {
    scheme: &'static str,
    userinfo: Option<&'a str>,
    host_port: &'a str,
    /// Path, query and fragment, from the first delimiter on.
    tail: &'a str,
}

impl<'a> HttpRemote<'a> {
    fn parse(remote_url: &'a str) -> Option<Self> {
        let (scheme, rest) = if let Some(rest) = remote_url.strip_prefix("https://") {
            ("https://", rest)
        } else {
            ("http://", remote_url.strip_prefix("http://")?)
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let (userinfo, host_port) = match authority.rsplit_once('@') {
            Some((user, host)) => (Some(user), host),
            None => (None, authority),
        };
        Some(Self {
            scheme,
            userinfo,
            host_port,
            tail,
        })
    }
}

/// Whether a failed push failed to *authenticate*, as opposed to being refused
/// by a remote that heard it.
///
/// The distinction is the whole difference between two pieces of advice, and
/// the wrong one sends the user in a circle. A push that origin rejected is
/// usually a branch that moved, and fetching fixes it; a push that never got
/// past the credential exchange is a token that is missing, expired, or
/// unreadable, and no amount of fetching will change that. Both exit non-zero,
/// so the exit code cannot tell them apart — which is why
/// `classify_exec_failure` answered `NonZeroExit` for a `fatal: could not read
/// Password` and the user was told to sync again.
///
/// Matched on git's own wording rather than an exit code for the same reason
/// [`crate::domain::harness_failure`] exists: this is the one place the reading
/// is made, so the rest of the tree never has to guess at a string.
pub fn is_credential_failure(stderr: &str) -> bool {
    const SIGNATURES: [&str; 6] = [
        // No credential at all, and no terminal to ask on.
        "could not read Password",
        "could not read Username",
        "terminal prompts disabled",
        // A credential that was offered and rejected.
        "Authentication failed",
        "Invalid username or password",
        // The ssh half of the same problem.
        "Permission denied (publickey)",
    ];
    SIGNATURES.iter().any(|sig| stderr.contains(sig))
}

/// The environment that stops `git` asking a human anything.
///
/// Neither push-specific nor remote-specific, which is why it is here and not
/// beside one invocation: any `git` that touches `origin` can reach a
/// credential exchange, and none of Demeteo's has somewhere to ask. A run is
/// unattended by construction, and the desktop's git child has no tty either.
/// The cost of omitting it is not a slower command but a process that blocks
/// until something kills it.
///
/// Three keys because `GIT_TERMINAL_PROMPT=0` closes only git's *own* prompt.
/// Per `gitcredentials(7)` git consults credential **helpers** first, and a
/// default Git for Windows install writes `credential.helper = manager` into
/// the system config — GCM then answers with a **GUI** dialog that no terminal
/// setting suppresses. `GCM_INTERACTIVE` and `GCM_GUI_PROMPT` are what it reads
/// instead.
pub fn unattended_env() -> std::collections::BTreeMap<String, String> {
    [
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GCM_INTERACTIVE", "false"),
        ("GCM_GUI_PROMPT", "0"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

#[cfg(test)]
#[path = "../../tests/domain/git_push.rs"]
mod tests;
