use crate::application::providers::sanitize_host;

#[test]
fn test_sanitize_host() {
    assert_eq!(
        sanitize_host("https://gitlab.stvcloud.dev/prototype/spectacular.git"),
        "gitlab.stvcloud.dev"
    );
    assert_eq!(
        sanitize_host("http://gitlab.company.com:8080/path"),
        "gitlab.company.com:8080"
    );
    assert_eq!(sanitize_host("gitlab.company.com"), "gitlab.company.com");
    assert_eq!(
        sanitize_host("   https://api.github.com   "),
        "api.github.com"
    );
}
