//! `slug_from_description` keeps any alphanumeric char, so its input reaches
//! the cap carrying multi-byte letters; the cap is in bytes and the cut must
//! still land between chars.

use super::*;

fn assert_well_formed(slug: &str) {
    assert!(!slug.is_empty(), "{slug:?}");
    assert!(slug.len() <= MAX_SLUG_LEN, "{} bytes: {slug:?}", slug.len());
    assert!(!slug.ends_with('-'), "{slug:?}");
}

#[test]
fn a_long_ascii_description_is_cut_at_its_last_word_as_before() {
    let slug = slug_from_description(
        "Add a retry policy to the remote runner so that flaky SSH connections recover",
    );
    assert_eq!(slug, "add-a-retry-policy-to-the-remote-runner-so-that");
    assert_well_formed(&slug);
}

#[test]
fn three_byte_chars_straddling_the_cap_are_dropped_whole() {
    let slug = slug_from_description(&"日".repeat(17));
    assert_eq!(slug, "日".repeat(16));
    assert_well_formed(&slug);

    let slug = slug_from_description(&format!("a{}", "日".repeat(20)));
    assert_eq!(slug, format!("a{}", "日".repeat(16)));
    assert_well_formed(&slug);
}

#[test]
fn a_two_byte_char_straddling_the_cap_is_dropped_whole() {
    let slug = slug_from_description(&format!("a{}", "é".repeat(30)));
    assert_eq!(slug, format!("a{}", "é".repeat(24)));
    assert_well_formed(&slug);
}

#[test]
fn a_cut_after_a_hyphen_does_not_leave_it_trailing() {
    let slug = slug_from_description(&format!("{} 日日", "a".repeat(47)));
    assert_eq!(slug, "a".repeat(47));
    assert_well_formed(&slug);
}
