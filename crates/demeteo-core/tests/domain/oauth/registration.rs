// Tests extracted from `src/domain/oauth/registration.rs` (mirrored-tests
// convention). `super` = `domain::oauth::registration`.

use super::*;

fn ok(uri: &str) -> bool {
    validate_registration("client", &[uri.to_string()]).is_ok()
}

#[test]
fn accepts_the_three_native_app_shapes() {
    assert!(ok("http://127.0.0.1:5173/callback"));
    assert!(ok("http://localhost/cb"));
    assert!(ok("http://[::1]:8080/cb"));
    assert!(ok("https://app.example.com/oauth/callback"));
    assert!(ok("com.example.app:/oauth2redirect"));
}

#[test]
fn refuses_everything_else() {
    for bad in [
        "",
        "http://example.com/cb",
        "http://127.0.0.1.evil.com/cb",
        "http://localhost@evil.com/cb",
        "https://user:pw@example.com/cb",
        "https:///cb",
        "https://example.com/cb#frag",
        "javascript:alert(1)",
        "data:text/html,x",
        "file:///etc/passwd",
        "myapp:/cb",
        "no-scheme",
        "://x",
        "1http://127.0.0.1/cb",
        "https://exa mple.com/cb",
        "https://example.com/cb\n",
    ] {
        assert!(!ok(bad), "{bad:?} must be refused");
    }
}

#[test]
fn bounds_the_name_and_the_list() {
    let uri = vec!["http://127.0.0.1/cb".to_string()];
    assert_eq!(
        validate_registration("  ", &uri),
        Err(RegistrationError::ClientName)
    );
    assert_eq!(
        validate_registration(&"n".repeat(MAX_CLIENT_NAME_CHARS + 1), &uri),
        Err(RegistrationError::ClientName)
    );
    for bad in ["a\nb", "a\u{202e}b", "a\u{200b}b"] {
        assert_eq!(
            validate_registration(bad, &uri),
            Err(RegistrationError::ClientName),
            "{bad:?}"
        );
    }
    assert_eq!(
        validate_registration("c", &[]),
        Err(RegistrationError::RedirectUriCount)
    );
    assert_eq!(
        validate_registration("c", &vec![uri[0].clone(); MAX_REDIRECT_URIS + 1]),
        Err(RegistrationError::RedirectUriCount)
    );
    assert!(validate_registration(&"n".repeat(MAX_CLIENT_NAME_CHARS), &uri).is_ok());
}
