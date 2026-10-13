//! One side of a diff, reachable without a repository under it. `super` is
//! `crate::domain::file_at_ref`.

use super::*;

const OID: &str = "45b983be36b73c0788dc9cbcb76cbb80fc7bb057";

fn entry(mode: &str, kind: &str, name: &str) -> String {
    format!("{mode} {kind} {OID}\t{name}\0")
}

#[test]
fn a_file_at_the_revision_is_its_blob() {
    assert_eq!(
        classify(Ok(&entry("100644", "blob", "a/b.txt")), "HEAD", "a/b.txt"),
        Ok(FileAtRef::Blob {
            oid: OID.to_string()
        }),
    );
}

#[test]
fn an_empty_listing_is_a_path_the_range_added_or_deleted() {
    assert_eq!(classify(Ok(""), "HEAD", "new.txt"), Ok(FileAtRef::Absent));
}

#[test]
fn an_unknown_revision_is_an_error_not_an_empty_file() {
    let verdict = classify(
        Err("fatal: Not a valid object name 0123abcd"),
        "0123abcd",
        "a/b.txt",
    );
    let err = verdict.expect_err("an unresolvable revision must surface");
    assert!(err.contains("0123abcd"), "{err}");
    assert!(err.contains("Not a valid object name"), "{err}");
}

#[test]
fn a_directory_that_is_not_a_repository_is_an_error() {
    assert!(classify(Err("fatal: not a git repository"), "HEAD", "a.txt").is_err());
}

#[test]
fn only_an_exact_name_match_counts() {
    let children = entry("100644", "blob", "a/b.txt");
    assert_eq!(classify(Ok(&children), "HEAD", "a/"), Ok(FileAtRef::Absent));
    assert_eq!(
        classify(Ok(&children), "HEAD", "a/b"),
        Ok(FileAtRef::Absent)
    );
}

#[test]
fn a_name_with_spaces_and_tabs_survives_the_nul_framing() {
    let name = "dir with space/odd\tname.txt";
    assert!(matches!(
        classify(Ok(&entry("100644", "blob", name)), "HEAD", name),
        Ok(FileAtRef::Blob { .. })
    ));
}

#[test]
fn a_symlink_is_shown_as_the_blob_git_stores_for_it() {
    assert!(matches!(
        classify(Ok(&entry("120000", "blob", "link")), "HEAD", "link"),
        Ok(FileAtRef::Blob { .. })
    ));
}

#[test]
fn a_directory_or_submodule_at_the_path_is_an_error() {
    assert!(classify(Ok(&entry("040000", "tree", "a")), "HEAD", "a").is_err());
    assert!(classify(
        Ok(&entry("160000", "commit", "vendor/x")),
        "HEAD",
        "vendor/x"
    )
    .is_err());
}

#[test]
fn a_malformed_entry_is_an_error_rather_than_a_guess() {
    assert!(classify(Ok("garbage\tf.txt\0"), "HEAD", "f.txt").is_err());
    assert!(classify(Ok("100644 blob not-hex\tf.txt\0"), "HEAD", "f.txt").is_err());
}
