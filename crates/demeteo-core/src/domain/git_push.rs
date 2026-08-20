//! What a push to `origin` needs before it runs, and how to read one that
//! failed. See [`crate::domain`].
//!
//! Both questions are answered from text — a remote URL, git's stderr — and
//! neither needs a port, which is the whole reason they live here rather than
//! inside the `async fn` that pushes.

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
    let rest = remote_url
        .strip_prefix("https://")
        .or_else(|| remote_url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let (userinfo, host) = match authority.rsplit_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, authority),
    };
    // A colon in the userinfo is a password already in hand.
    if userinfo.is_some_and(|u| u.contains(':')) {
        return None;
    }
    let host = host.split(':').next().unwrap_or(host);
    (!host.is_empty()).then_some(host)
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

#[cfg(test)]
#[path = "../../tests/domain/git_push.rs"]
mod tests;
