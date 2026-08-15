//! What a reviewing step is told about where its diff starts.
//!
//! No doubles: the block is pure over the fork point an adapter already
//! resolved, so the whole placement rule is reachable without an
//! `ExecutionDriver` (AGENTS.md §3).

use super::*;

const BRANCH: &str = "demeteo/features/f-42";
const SHA: &str = "abc1234";

#[test]
fn a_resolved_base_names_the_commit_and_the_range() {
    let block = review_base_section(Some(SHA), BRANCH);
    assert!(block.contains(SHA), "the commit must be named: {block}");
    assert!(
        block.contains(&format!("git diff --name-status {SHA}..HEAD")),
        "the range has to arrive as a runnable command, not as prose: {block}"
    );
    assert!(
        block.contains(BRANCH),
        "the agent must be told which branch the commit belongs to: {block}"
    );
}

/// The trap the block exists to close: implementation work is committed, so a
/// bare `git diff` succeeds and returns nothing. An agent that reads that as
/// "no change" writes a passing review of an unread branch.
#[test]
fn a_resolved_base_warns_that_a_bare_diff_is_empty() {
    let block = review_base_section(Some(SHA), BRANCH);
    assert!(
        block.contains("bare `git diff` is normally empty"),
        "the empty-diff warning is the load-bearing sentence: {block}"
    );
}

/// Neither rendering may invite a guess — a wrong base is the failure mode
/// that produced this module, and it is silent in both directions.
#[test]
fn neither_rendering_lets_the_agent_guess_a_base_branch() {
    for fork_point in [Some(SHA), None] {
        let block = review_base_section(fork_point, BRANCH);
        assert!(
            block.contains("guess"),
            "every rendering must forbid guessing a base, got: {block}"
        );
    }
}

/// An unresolved fork point is *rendered*, not suppressed. Saying nothing
/// leaves the agent in exactly the state that made a wrong base possible.
#[test]
fn an_unresolved_base_gives_a_procedure_instead_of_a_range() {
    let block = review_base_section(None, BRANCH);
    assert!(!block.is_empty(), "silence is not an option here");
    assert!(
        block.contains("git log --oneline --decorate"),
        "the fallback must hand over a command that works without a base: {block}"
    );
    assert!(
        !block.contains("..HEAD"),
        "a range must not appear when there is no base to anchor it: {block}"
    );
}

/// The block is authoritative text an agent acts on, so it may not describe a
/// range only one origin produces: a run cut from a pull request head is
/// measured against what that PR merges into, not against the project default.
#[test]
fn neither_rendering_calls_the_left_side_the_default_branch() {
    for fork_point in [Some(SHA), None] {
        let block = review_base_section(fork_point, BRANCH);
        assert!(
            !block.contains("default branch"),
            "the left side is this run's base, which is the default branch only \
             for a run that started there: {block}"
        );
    }
}

#[test]
fn a_template_naming_the_token_places_the_block_itself() {
    let placed = place_review_base(
        Some(SHA),
        BRANCH,
        StepCapability::Verify,
        "review it.\n{{review_base_section}}\nthen report.",
    );
    assert!(placed.prefix.is_empty(), "the template placed it");
    assert!(placed.bound.contains(SHA));
}

/// The reason the fallback exists: prompt templates live in the DB and are
/// user-authored, so no template on an existing install names a token
/// introduced today. A verify step that never heard of the token still gets
/// the block.
#[test]
fn a_reviewing_capability_gets_the_block_without_naming_it() {
    for capability in [StepCapability::Verify, StepCapability::ReadOnly] {
        let placed = place_review_base(Some(SHA), BRANCH, capability, "review it.");
        assert!(
            placed.bound.is_empty(),
            "{capability:?} named no token, so nothing binds"
        );
        assert!(
            placed.prefix.contains(SHA),
            "{capability:?} reviews work, so the block must reach it anyway"
        );
    }
}

/// The other half of that rule. A step that writes code is not paying for a
/// block telling it how to read one, and only an explicit token changes that.
#[test]
fn a_non_reviewing_capability_pays_nothing_unless_it_asks() {
    for capability in [StepCapability::Implement, StepCapability::Artifacts] {
        let silent = place_review_base(Some(SHA), BRANCH, capability, "build it.");
        assert!(silent.bound.is_empty() && silent.prefix.is_empty());
        assert!(
            !needs_review_base(capability, "build it."),
            "{capability:?} must not trigger the merge-base round trip"
        );

        let asked = place_review_base(
            Some(SHA),
            BRANCH,
            capability,
            "fix it.\n{{review_base_section}}",
        );
        assert!(
            asked.bound.contains(SHA),
            "{capability:?} asked for the block by name, so it gets it"
        );
    }
}

/// Placement decides *where*, never *whether the bytes differ* — two renders
/// that drifted would be two blocks to keep in step.
#[test]
fn both_placements_carry_the_same_bytes() {
    let bound = place_review_base(
        Some(SHA),
        BRANCH,
        StepCapability::Verify,
        "{{review_base_section}}",
    );
    let prefixed = place_review_base(Some(SHA), BRANCH, StepCapability::Verify, "review it.");
    assert_eq!(bound.bound, prefixed.prefix);
}
