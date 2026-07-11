//! Defence-in-depth secret scrubbing for anything that leaves the
//! process as human-/machine-readable text — the runner's append-only
//! event log (`run_events`) and its away-channel notifications
//! (docs/REMOTE_EXECUTION_PLAN.md M7.2, docs/REMOTE_EXECUTION.md §6).
//!
//! The credential design already keeps the git PAT out of argv, git
//! config, and the URL (M4.3's askpass helper), so nothing is *supposed*
//! to reach these sinks with a token in it. This is the belt-and-braces
//! layer for the paths that stringify a foreign error we don't control:
//! a git transport error, a provider HTTP body, a clone failure — any of
//! which could echo back a credential-bearing URL or a token-shaped
//! string. Scrub at the sink so a single missed redaction upstream can't
//! become a token written to disk or POSTed to a webhook.
//!
//! Deliberately dependency-free (no regex): the patterns are fixed and a
//! hand-written scan has no ReDoS surface and is trivial to audit.

use std::borrow::Cow;

/// The text a matched secret is replaced with.
const MASK: &str = "***";

/// Known standalone-token prefixes. A run of token characters
/// (`[A-Za-z0-9_-]`) beginning with any of these is masked wholesale.
/// Covers GitHub classic + fine-grained PATs and all four OAuth token
/// classes, plus GitLab personal/project access tokens.
const TOKEN_PREFIXES: &[&str] = &[
    "github_pat_", // GitHub fine-grained PAT
    "ghp_",        // GitHub classic PAT
    "gho_",        // GitHub OAuth token
    "ghu_",        // GitHub user-to-server token
    "ghs_",        // GitHub server-to-server token
    "ghr_",        // GitHub refresh token
    "glpat-",      // GitLab personal access token
    "glptt-",      // GitLab pipeline trigger token
];

/// A token character — the alphabet GitHub/GitLab tokens draw from.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Redact any credential-shaped substring from `input`.
///
/// Two independent passes, each conservative enough not to mangle
/// ordinary prose:
/// 1. **URL userinfo** — `scheme://user:secret@host` has its `:secret`
///    span masked (the password half of HTTP basic-auth, which is where
///    an embedded PAT lands). A userinfo with no `:` (the token-free
///    `x-access-token@host` form M4.3 emits) is left untouched — there's
///    no secret there.
/// 2. **Bare tokens** — a `[A-Za-z0-9_-]` run starting with a known
///    provider token prefix ([`TOKEN_PREFIXES`]) is masked whole.
///
/// Returns `Cow::Borrowed` when nothing matched, so the common
/// (already-clean) path allocates nothing.
pub fn scrub_secrets(input: &str) -> Cow<'_, str> {
    let after_urls = scrub_url_userinfo(input);
    match scrub_token_prefixes(&after_urls) {
        Cow::Owned(s) => Cow::Owned(s),
        // Token pass changed nothing: propagate whatever the URL pass
        // produced (borrowed if *it* also changed nothing).
        Cow::Borrowed(_) => after_urls,
    }
}

/// Pass 1: mask the password half of any `://user:secret@host` userinfo.
fn scrub_url_userinfo(input: &str) -> Cow<'_, str> {
    if !input.contains("://") {
        return Cow::Borrowed(input);
    }
    let bytes = input;
    let mut out = String::new();
    let mut changed = false;
    let mut rest = bytes;
    while let Some(scheme_pos) = rest.find("://") {
        let userinfo_start = scheme_pos + 3;
        // The userinfo ends at the first '@' that comes before the next
        // '/', '?', '#', or whitespace (the authority terminators). No
        // such '@' means this URL has no userinfo — nothing to scrub.
        let authority = &rest[userinfo_start..];
        let at = authority.find('@');
        let boundary =
            authority.find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace());
        let has_userinfo = match (at, boundary) {
            (Some(a), Some(b)) => a < b,
            (Some(_), None) => true,
            _ => false,
        };
        if !has_userinfo {
            // Copy through the scheme marker and continue scanning after it.
            out.push_str(&rest[..userinfo_start]);
            rest = &rest[userinfo_start..];
            continue;
        }
        let at = at.unwrap();
        let userinfo = &authority[..at];
        out.push_str(&rest[..userinfo_start]);
        if let Some(colon) = userinfo.find(':') {
            // Keep the username, mask the password half.
            out.push_str(&userinfo[..colon + 1]);
            out.push_str(MASK);
            changed = true;
        } else {
            // Token-free userinfo (e.g. `x-access-token@`) — keep as-is.
            out.push_str(userinfo);
        }
        out.push('@');
        rest = &authority[at + 1..];
    }
    out.push_str(rest);
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(input)
    }
}

/// Pass 2: mask any token-char run beginning with a known provider prefix.
fn scrub_token_prefixes(input: &str) -> Cow<'_, str> {
    let has_candidate = TOKEN_PREFIXES.iter().any(|p| input.contains(p));
    if !has_candidate {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut idx = 0;
    let mut changed = false;
    while idx < input.len() {
        let tail = &input[idx..];
        if let Some(prefix) = TOKEN_PREFIXES.iter().find(|p| tail.starts_with(**p)) {
            // Consume the whole token-char run (prefix included).
            let run_len = tail
                .char_indices()
                .find(|(_, c)| !is_token_char(*c))
                .map(|(i, _)| i)
                .unwrap_or(tail.len());
            // Only treat it as a secret if there's something after the
            // prefix — a lone `ghp_` in prose isn't a token.
            if run_len > prefix.len() {
                out.push_str(MASK);
                idx += run_len;
                changed = true;
                continue;
            }
        }
        // Advance one full char (never split a UTF-8 boundary).
        let step = tail.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&tail[..step]);
        idx += step;
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(input)
    }
}

#[cfg(test)]
#[path = "../../tests/shared/secret_scrub.rs"]
mod tests;
