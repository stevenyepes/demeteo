use super::*;

fn github() -> ListTarget<'static> {
    ListTarget {
        kind: "github",
        host: "api.github.com",
    }
}

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn classify(status: u16, body: &str, hs: &[(&str, &str)]) -> Result<(), MrListError> {
    let hs = headers(hs);
    classify_list_response(
        github(),
        ListResponse {
            status,
            body,
            headers: &hs,
        },
    )
}

#[test]
fn success_passes_through() {
    assert!(classify(200, "[]", &[]).is_ok());
    assert!(classify(299, "[]", &[]).is_ok());
}

#[test]
fn unauthorized_is_never_ok() {
    assert_eq!(
        classify(401, "Bad credentials", &[]),
        Err(MrListError::Unauthorized {
            provider: "github".into(),
            host: "api.github.com".into(),
            status: 401,
        })
    );
}

#[test]
fn forbidden_without_quota_headers_reads_as_a_token_problem() {
    assert_eq!(
        classify(403, "Resource not accessible", &[]),
        Err(MrListError::Unauthorized {
            provider: "github".into(),
            host: "api.github.com".into(),
            status: 403,
        })
    );
}

#[test]
fn too_many_requests_carries_retry_after() {
    assert_eq!(
        classify(429, "slow down", &[("Retry-After", "42")]),
        Err(MrListError::RateLimited {
            host: "api.github.com".into(),
            retry_after: Some(42),
        })
    );
}

#[test]
fn rate_limited_without_a_usable_retry_after_still_rate_limits() {
    // The HTTP-date form is deliberately unparsed; the failure stays a rate
    // limit, it just cannot say for how long.
    assert_eq!(
        classify(429, "", &[("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT")]),
        Err(MrListError::RateLimited {
            host: "api.github.com".into(),
            retry_after: None,
        })
    );
}

#[test]
fn forbidden_with_exhausted_quota_is_a_rate_limit_not_a_bad_token() {
    assert_eq!(
        classify(
            403,
            "API rate limit exceeded",
            &[("X-RateLimit-Remaining", "0")]
        ),
        Err(MrListError::RateLimited {
            host: "api.github.com".into(),
            retry_after: None,
        })
    );
}

#[test]
fn forbidden_with_quota_remaining_is_a_token_problem() {
    assert_eq!(
        classify(403, "nope", &[("X-RateLimit-Remaining", "4999")]),
        Err(MrListError::unauthorized(github(), 403))
    );
}

#[test]
fn server_error_keeps_the_providers_own_words() {
    assert_eq!(
        classify(500, "  <html>boom</html>  ", &[]),
        Err(MrListError::Http {
            host: "api.github.com".into(),
            status: Some(500),
            body: "<html>boom</html>".into(),
        })
    );
}

#[test]
fn a_body_longer_than_the_cap_is_truncated() {
    let huge = "x".repeat(5_000);
    let Err(MrListError::Http { body, .. }) = classify(502, &huge, &[]) else {
        panic!("a 502 must not classify as anything but Http");
    };
    assert_eq!(body.chars().count(), BODY_LIMIT + 1);
    assert!(body.ends_with('…'));
}

#[test]
fn a_multibyte_body_is_cut_on_a_character_boundary() {
    let huge = "é".repeat(5_000);
    let Err(MrListError::Http { body, .. }) = classify(502, &huge, &[]) else {
        panic!("a 502 must not classify as anything but Http");
    };
    assert_eq!(body.chars().count(), BODY_LIMIT + 1);
}

#[test]
fn redirects_do_not_coerce_to_success() {
    // 3xx is the status range the sibling `fetch_mr_state` path swallows as
    // `open`; the listing must not inherit that.
    assert!(classify(301, "moved", &[]).is_err());
    assert!(classify(304, "", &[]).is_err());
}

/// The IPC contract with `src/lib/pullRequests.ts`. Its own test feeds these
/// exact strings to `asPullRequestListFailure`; if a rename lands on one side
/// only, one of the two goes red.
#[test]
fn serialized_shape_is_the_wire_contract() {
    let json = |e: &MrListError| serde_json::to_string(e).expect("MrListError is serializable");

    assert_eq!(json(&MrListError::NoProvider), r#"{"kind":"no-provider"}"#);
    assert_eq!(
        json(&MrListError::Unauthorized {
            provider: "github".into(),
            host: "api.github.com".into(),
            status: 401,
        }),
        r#"{"kind":"unauthorized","provider":"github","host":"api.github.com","status":401}"#
    );
    assert_eq!(
        json(&MrListError::RateLimited {
            host: "gitlab.com".into(),
            retry_after: Some(30),
        }),
        r#"{"kind":"rate-limited","host":"gitlab.com","retry_after":30}"#
    );
    assert_eq!(
        json(&MrListError::Http {
            host: "api.github.com".into(),
            status: Some(500),
            body: "boom".into(),
        }),
        r#"{"kind":"http","host":"api.github.com","status":500,"body":"boom"}"#
    );
    assert_eq!(
        json(&MrListError::other("", "database query failed")),
        r#"{"kind":"http","host":"","status":null,"body":"database query failed"}"#
    );
}
