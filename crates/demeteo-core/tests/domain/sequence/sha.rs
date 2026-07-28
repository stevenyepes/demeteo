// Tests extracted from `crates/demeteo-core/src/domain/sequence/sha.rs` (mirrored-tests convention). `super` = that module.

use super::*;

/// Git prints a trailing newline. Three call sites used to trim it
/// themselves — the rollback anchor, the task loop's checkpoint pin, and the
/// pre-turn HEAD — and one of them forgetting would interpolate a two-line
/// operand into the next command.
#[test]
fn rev_parse_output_arrives_trimmed() {
    assert_eq!(Sha::from_output("  abc123 \n").as_str(), "abc123");
}

/// `rev-parse` can succeed with nothing to say, and the callers disagree
/// about what that means — so the type reports it rather than deciding it.
#[test]
fn an_empty_answer_stays_empty_rather_than_being_rejected() {
    assert!(Sha::from_output("\n  \n").is_empty());
    assert!(!Sha::from_output("abc123").is_empty());
}
