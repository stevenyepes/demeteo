use super::{attributed, ATTRIBUTION};

#[test]
fn the_report_survives_and_the_attribution_follows_it() {
    let posted = attributed("## Findings\n\nOne blocking issue.");

    assert!(posted.starts_with("## Findings\n\nOne blocking issue."));
    assert!(posted.ends_with(ATTRIBUTION));
}

#[test]
fn a_report_that_already_ends_in_blank_lines_does_not_grow_them() {
    assert_eq!(attributed("done\n\n\n"), attributed("done"));
}
