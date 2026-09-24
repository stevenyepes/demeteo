// Tests for `domain::oauth::resource_matches`. `super` = `domain::oauth`.

use super::*;

const CANONICAL: &str = "http://127.0.0.1:8765";

#[test]
fn the_canonical_origin_matches() {
    assert!(resource_matches(CANONICAL, CANONICAL));
}

#[test]
fn the_trailing_slash_form_a_ts_sdk_client_sends_matches() {
    assert!(resource_matches("http://127.0.0.1:8765/", CANONICAL));
}

#[test]
fn any_other_spelling_is_refused() {
    for requested in [
        "",
        "http://127.0.0.1:8765//",
        "http://127.0.0.1:8765/mcp",
        "http://127.0.0.1:8766",
        "http://127.0.0.1",
        "http://localhost:8765",
        "https://127.0.0.1:8765",
    ] {
        assert!(
            !resource_matches(requested, CANONICAL),
            "{requested:?} must not match {CANONICAL:?}"
        );
    }
}
