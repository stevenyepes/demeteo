use super::*;

#[test]
fn the_default_source_resolves_what_the_credential_cache_holds() {
    crate::credential_cache::set("prov-default-source", "tok-default-source-42");

    let pat = KeyringCredentialSource.pat_for("prov-default-source", "/anywhere");

    assert_eq!(pat.as_deref(), Ok("tok-default-source-42"));
}

#[test]
fn the_default_source_answers_the_same_for_every_repo_dir() {
    crate::credential_cache::set("prov-default-source-dirs", "tok-default-dirs-77");

    assert_eq!(
        KeyringCredentialSource.pat_for("prov-default-source-dirs", "/a"),
        KeyringCredentialSource.pat_for("prov-default-source-dirs", "/b"),
    );
}
