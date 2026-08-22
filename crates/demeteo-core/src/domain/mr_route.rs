//! Which configured provider a request URL is served by. See [`crate::domain`].
//!
//! A project holds one `repositories` row per repository, each naming its own
//! provider, and the review queue aggregates across all of them on purpose. So
//! "the project's provider" is not a thing: reading one request in full has to
//! resolve the provider from the request, not from whichever repository sorted
//! first. Sending a `github.com` row's GET to a GitHub Enterprise host answers
//! 404 with a token that was never scoped for it, and a GitHub row dispatched
//! at a GitLab provider is refused by the URL parser before it leaves the
//! process — both silently, because the row simply stays unenriched.
//!
//! The host is the whole of the match. A detail read takes only the provider's
//! kind, its host and its token; `owner/repo` comes from the URL itself. Two
//! repositories on one host therefore resolve to the same request whichever of
//! them is picked, and no repo-path comparison would change the call.

/// Whether the provider instance at `provider_host` is the one serving
/// `mr_url`.
pub fn serves_request(provider_host: &str, mr_url: &str) -> bool {
    match request_host(mr_url) {
        Some(host) => canonical(provider_host) == canonical(host),
        None => false,
    }
}

/// The host a request URL names, or `None` when it names none.
fn request_host(mr_url: &str) -> Option<&str> {
    let authority = mr_url
        .split_once("://")
        .map_or(mr_url, |(_, rest)| rest)
        .split('/')
        .next()?;
    // Credentials and a port belong to the request, not to the identity of the
    // host a provider row was configured with.
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split_once(':').map_or(host, |(h, _)| h);
    (!host.is_empty()).then_some(host)
}

/// One spelling per host, so the comparison is not defeated by the three names
/// GitHub answers to and by case.
///
/// A provider row's `host` is user-entered and an empty one means the default
/// GitHub, which is the same host `github.com` and `api.github.com` name.
fn canonical(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    match host.as_str() {
        "" | "api.github.com" => "github.com".to_string(),
        _ => host,
    }
}

#[cfg(test)]
#[path = "../../tests/domain/mr_route.rs"]
mod tests;
