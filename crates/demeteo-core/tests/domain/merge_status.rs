// The porcelain conflict parser. `super` = `domain::merge_status`.
//
// First coverage this has had: it existed as three byte-identical copies, none
// of them tested, and it is read on both the local and the SSH transport.

use super::*;

fn kinds(porcelain: &str) -> Vec<(String, String)> {
    parse_unmerged(porcelain)
        .into_iter()
        .map(|f| (f.path, f.kind))
        .collect()
}

#[test]
fn every_conflict_code_maps_to_the_kind_the_ui_names() {
    // These strings cross to the frontend verbatim. A rename here is a
    // frontend change, not a cleanup.
    let porcelain = "\
UU src/both.rs
AA src/added-both.rs
DD src/gone.rs
UA src/theirs.rs
AU src/ours.rs
UD src/deleted-theirs.rs
DU src/deleted-ours.rs";
    assert_eq!(
        kinds(porcelain),
        vec![
            ("src/both.rs".to_string(), "both-modified".to_string()),
            ("src/added-both.rs".to_string(), "both-modified".to_string()),
            ("src/gone.rs".to_string(), "both-modified".to_string()),
            ("src/theirs.rs".to_string(), "added-by-them".to_string()),
            ("src/ours.rs".to_string(), "added-by-us".to_string()),
            (
                "src/deleted-theirs.rs".to_string(),
                "deleted-by-them".to_string()
            ),
            (
                "src/deleted-ours.rs".to_string(),
                "deleted-by-us".to_string()
            ),
        ]
    );
}

#[test]
fn an_ordinary_dirty_tree_yields_no_conflicts() {
    let porcelain = "\
 M src/edited.rs
?? src/new.rs
A  src/staged.rs
 D src/removed.rs";
    assert!(parse_unmerged(porcelain).is_empty(), "{porcelain}");
}

#[test]
fn a_line_too_short_to_carry_a_code_is_skipped_rather_than_panicking() {
    // A slice of `line[..2]` on a one-character line would panic; the guard
    // is what keeps a truncated read from taking the run down.
    assert!(parse_unmerged("U\n\nX\nUU\n").is_empty());
}

#[test]
fn leading_whitespace_is_trimmed_before_the_code_is_read() {
    // `git status` pads the XY column; a leading space must not shift the
    // read by one and turn `UU` into ` U`.
    assert_eq!(
        kinds("   UU src/both.rs"),
        vec![("src/both.rs".to_string(), "both-modified".to_string())]
    );
}

#[test]
fn the_path_starts_at_byte_three_and_is_trimmed() {
    assert_eq!(
        kinds("UU   src/spaced.rs   "),
        vec![("src/spaced.rs".to_string(), "both-modified".to_string())]
    );
}

#[test]
fn crlf_line_endings_do_not_leak_into_the_path() {
    // `lines()` already strips `\r`; nothing normalises it separately.
    assert_eq!(
        kinds("UU src/both.rs\r\nUA src/theirs.rs\r\n"),
        vec![
            ("src/both.rs".to_string(), "both-modified".to_string()),
            ("src/theirs.rs".to_string(), "added-by-them".to_string()),
        ]
    );
}
