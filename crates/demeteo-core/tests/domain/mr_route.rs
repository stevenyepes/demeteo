//! Which provider a request URL is routed at, with no provider under it.
//! `super` is `crate::domain::mr_route`.

use super::*;

#[test]
fn a_request_is_served_by_the_provider_whose_host_it_names() {
    assert!(serves_request(
        "github.com",
        "https://github.com/stvcloud/demeteo/pull/118"
    ));
    assert!(serves_request(
        "ghes.corp.com",
        "https://ghes.corp.com/stvcloud/demeteo/pull/118"
    ));
    assert!(serves_request(
        "gitlab.com",
        "https://gitlab.com/stvcloud/demeteo/-/merge_requests/42"
    ));
}

/// The failure this exists for: an enterprise instance configured first in a
/// project that also tracks a `github.com` repository. Routing the second
/// repository's rows at the first one's host sends the GET somewhere the token
/// does not reach, and the row stays undecided forever with nothing said.
#[test]
fn a_request_on_another_host_is_not_this_provider_s() {
    assert!(!serves_request(
        "ghes.corp.com",
        "https://github.com/stvcloud/demeteo/pull/118"
    ));
    assert!(!serves_request(
        "github.com",
        "https://gitlab.com/stvcloud/demeteo/-/merge_requests/42"
    ));
}

/// A provider row's host is typed by a person, and an empty one is the default
/// GitHub — the same host `api.github.com` names.
#[test]
fn one_host_spelled_three_ways_is_still_one_host() {
    for host in [
        "",
        "GitHub.com",
        "api.github.com",
        "www.github.com",
        "github.com.",
    ] {
        assert!(
            serves_request(host, "https://github.com/stvcloud/demeteo/pull/118"),
            "{host} names github.com"
        );
    }
}

#[test]
fn a_url_naming_no_host_is_served_by_nobody() {
    for url in ["", "/stvcloud/demeteo/pull/118", "https:///pull/118"] {
        assert!(!serves_request("github.com", url), "{url:?} names no host");
    }
}

/// The URL is the request as the provider published it; the port and any
/// embedded credentials are not part of which instance it belongs to.
#[test]
fn a_port_or_a_credential_does_not_move_a_request_to_another_provider() {
    assert!(serves_request(
        "gitlab.corp.com",
        "https://gitlab.corp.com:8443/team/app/-/merge_requests/3"
    ));
    assert!(serves_request(
        "gitlab.corp.com",
        "https://ci-token:secret@gitlab.corp.com/team/app/-/merge_requests/3"
    ));
}
