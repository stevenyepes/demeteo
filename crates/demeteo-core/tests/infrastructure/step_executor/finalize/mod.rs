// Tests for `steps/finalize/mod.rs` (mirrored-tests convention).
// `super` = that module.

use super::*;

/// The load-bearing safety property of the whole design: the finalize agent
/// runs without a shell, so it *cannot* invoke `gh`/`glab`/`curl` to open the
/// PR itself — Demeteo does that, through the provider's HTTP API. This is
/// enforcement, not instruction, and this test pins it: widening the finalize
/// capability (or letting a workflow's `allow_shell` reach it) breaks here.
#[test]
fn the_finalize_agent_has_no_shell() {
    use crate::adapters::agent::claude_code::disallowed_tools_for;
    use crate::domain::permission::resolve_profile;

    let profile = resolve_profile(ExecutionDriver::finalize_capability(), false, false);
    assert!(
        !profile.execute.is_allow(),
        "the finalize agent must never be allowed to execute commands"
    );
    assert!(
        !profile.network.is_allow(),
        "the finalize agent has no reason to reach the network"
    );

    let denied = disallowed_tools_for(&profile);
    assert!(
        denied.contains(&"Bash"),
        "Bash must be denied to the finalize agent — without that, nothing stops it \
         from running `gh pr create`. Denied set was: {denied:?}"
    );
}

// ── Authored::commit_message ─────────────────────────────────────────────

#[test]
fn commit_message_joins_subject_and_body_with_a_blank_line() {
    let a = Authored {
        commit_subject: "feat(api): add retries".to_string(),
        commit_body: "The upstream flakes under load.".to_string(),
        pr_title: "t".to_string(),
        pr_body: "b".to_string(),
    };
    assert_eq!(
        a.commit_message(),
        "feat(api): add retries\n\nThe upstream flakes under load."
    );
}

#[test]
fn commit_message_omits_the_blank_line_when_there_is_no_body() {
    let a = Authored {
        commit_subject: "feat(api): add retries".to_string(),
        commit_body: "   ".to_string(),
        pr_title: "t".to_string(),
        pr_body: "b".to_string(),
    };
    assert_eq!(a.commit_message(), "feat(api): add retries");
}

// ── Authored::fallback ───────────────────────────────────────────────────
// The agent never answering must not throw away a completed feature. The
// work is committed and correct by this point; only its summary is missing.

#[test]
fn fallback_takes_the_first_five_words_of_the_feature_title() {
    let a = Authored::fallback("Add retry budget to the harness gate for flaky tests");
    assert_eq!(a.commit_subject, "chore: add retry budget to the");
    assert_eq!(
        a.pr_title,
        "Add retry budget to the harness gate for flaky tests"
    );
}

#[test]
fn fallback_truncates_a_long_subject_at_40_chars() {
    let a = Authored::fallback("Supercalifragilistic expialidocious antidisestablishmentarian");
    let subject = a.commit_subject.trim_start_matches("chore: ");
    assert!(
        subject.chars().count() <= 40,
        "fallback subject should be capped at 40 chars, got {} in {subject:?}",
        subject.chars().count()
    );
}

#[test]
fn fallback_never_produces_an_empty_subject() {
    let a = Authored::fallback("   ");
    assert_eq!(a.commit_subject, "chore: update");
}

// ── parse_authored ───────────────────────────────────────────────────────

use super::turn::parse_authored;

#[test]
fn parses_the_four_strings_from_a_fenced_json_answer() {
    let raw = "Here's the summary:\n```json\n{\n  \"commit_subject\": \"feat(api): add retries\",\n  \
               \"commit_body\": \"Upstream flakes.\",\n  \"pr_title\": \"Add retries to the API client\",\n  \
               \"pr_body\": \"## Why\\nIt flakes.\"\n}\n```";
    let a = parse_authored(raw).expect("should parse");
    assert_eq!(a.commit_subject, "feat(api): add retries");
    assert_eq!(a.commit_body, "Upstream flakes.");
    assert_eq!(a.pr_title, "Add retries to the API client");
    assert_eq!(a.pr_body, "## Why\nIt flakes.");
}

/// An answer that clearly *is* an answer, just missing one of the two title
/// fields, is worth more than a failed step: each stands in for the other.
#[test]
fn a_missing_commit_subject_falls_back_to_the_pr_title() {
    let a = parse_authored(r#"{"pr_title": "Add retries", "pr_body": "why"}"#).unwrap();
    assert_eq!(a.commit_subject, "Add retries");
    assert_eq!(a.pr_title, "Add retries");
}

#[test]
fn no_json_at_all_is_not_an_answer() {
    assert!(parse_authored("I couldn't figure out what changed, sorry.").is_none());
}

/// Well-formed JSON that says nothing is not an answer either — otherwise we
/// would squash the branch under an empty commit message.
#[test]
fn empty_subject_and_title_is_not_an_answer() {
    assert!(parse_authored(r#"{"pr_title": "  ", "commit_subject": ""}"#).is_none());
}
