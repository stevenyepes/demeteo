// Tests for the handoff extractor. `super` = `domain::sequence::handoff`.

use super::*;

#[test]
fn a_handoff_heading_carries_its_section_to_the_end_of_the_reply() {
    let reply =
        "## What changed\nAdded the port.\n\n## Handoff\nThe port trait is in ports/foo.rs; \
                 the adapter is a stub.\nTicket 2 fills it in.\n";
    assert_eq!(
        extract_handoff(reply).as_deref(),
        Some("The port trait is in ports/foo.rs; the adapter is a stub.\nTicket 2 fills it in.")
    );
}

#[test]
fn the_section_stops_at_the_next_heading() {
    let reply = "## Handoff\nuse the new helper\n\n## Test results\n3 passed\n";
    assert_eq!(
        extract_handoff(reply).as_deref(),
        Some("use the new helper")
    );
}

#[test]
fn the_last_handoff_wins_over_one_quoted_from_the_prompt() {
    let reply =
        "The prompt said:\n## Handoff\nwhat the next ticket needs\n\n## Handoff\nreal note\n";
    assert_eq!(extract_handoff(reply).as_deref(), Some("real note"));
}

#[test]
fn the_older_bullet_wording_is_read_too() {
    for reply in [
        "- Test results: 4 passed\n- Anything a later ticket needs to know: the cache key changed\n",
        "**Anything a later task needs to know**: the cache key changed\n",
        "### Anything a later ticket needs to know\nthe cache key changed\n",
        "Handoff: the cache key changed",
    ] {
        assert_eq!(
            extract_handoff(reply).as_deref(),
            Some("the cache key changed"),
            "{reply}"
        );
    }
}

#[test]
fn an_empty_or_dismissive_section_is_no_handoff() {
    for reply in [
        "## Handoff\n",
        "## Handoff\nnone\n",
        "## Handoff\nNone.\n\n## Next\n",
        "- Anything a later ticket needs to know: nothing\n",
        "## Handoff\n`n/a`\n",
        "no section here at all",
    ] {
        assert_eq!(extract_handoff(reply), None, "{reply}");
    }
}

#[test]
fn a_long_handoff_keeps_its_head_and_says_how_much_it_dropped() {
    let long = "x".repeat(MAX_HANDOFF_CHARS + 40);
    let out = extract_handoff(&format!("## Handoff\n{long}")).unwrap();
    assert!(out.starts_with(&"x".repeat(MAX_HANDOFF_CHARS)));
    assert!(
        out.ends_with("[40 more characters not carried forward]"),
        "{out}"
    );
}
