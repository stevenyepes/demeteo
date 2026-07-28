// Tests extracted from `crates/demeteo-core/src/domain/sequence/progress.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn contribution(refs: &[&str], decls: &[&str]) -> TaskContribution {
    TaskContribution {
        artifact_refs: strs(refs),
        satisfied_decls: strs(decls),
    }
}

/// A checkpoint payload, as a previous attempt's landed tasks recorded it.
fn payload(refs: &[&str], decls: &[&str]) -> CheckpointProduced {
    CheckpointProduced {
        artifact_refs: strs(refs),
        satisfied_decls: strs(decls),
    }
}

/// Declarations are a set, so their order out of the tally is not the order
/// in. Sort before asserting; nothing in the step reads them positionally.
fn sorted_decls(tally: &StepTally) -> Vec<String> {
    let mut decls = tally.produced().satisfied_decls;
    decls.sort();
    decls
}

/// A resumed attempt runs none of the tasks whose work the checkpoint
/// records, so the row is the only surviving evidence of what the step
/// produced. Starting empty here is what fails a step for a deliverable
/// that is already on disk, and starves its consumers of the refs.
#[test]
fn a_resumed_tally_starts_from_what_the_checkpoint_recorded() {
    let tally = StepTally::resuming(Some(&payload(&["a/1.md"], &["report"])));

    assert_eq!(tally.artifact_refs(), ["a/1.md".to_string()]);
    assert!(tally.satisfies("report"));
}

/// `None` is a pre-V36 row that cannot say what it produced. The tally holds
/// only what it was told — the *unknown* vs *empty* distinction is the
/// caller's to keep, and a tally that invented entries here would make it
/// unkeepable.
#[test]
fn a_tally_resuming_without_a_payload_starts_empty() {
    let tally = StepTally::resuming(None);

    assert!(tally.artifact_refs().is_empty());
    assert!(!tally.satisfies("report"));
    assert!(tally.landed().is_empty());
}

/// The landed prefix belongs to *this* attempt: it drives the worktree reset
/// and the merge a mid-list failure performs. Seeding it from the checkpoint
/// would offer up a previous attempt's commits — which are either already
/// merged or not in this worktree — as this attempt's prefix.
#[test]
fn a_resumed_tally_claims_none_of_the_previous_attempts_commits() {
    let tally = StepTally::resuming(Some(&payload(&["a/1.md"], &["report"])));

    assert!(tally.landed().is_empty());
}

/// Each task returns only what *it* emitted, so the step's totals exist only
/// because every fold adds to them. A fold that replaced instead of appended
/// would hand downstream just the last task's artifacts and fail the step
/// for deliverables an earlier task wrote.
#[test]
fn folding_a_contribution_keeps_every_earlier_one() {
    let mut tally = StepTally::resuming(Some(&payload(&["seed.md"], &["seeded"])));
    tally.fold(contribution(&["a/1.md"], &["notes"]));
    tally.fold(contribution(&["b/2.md"], &["report"]));

    assert_eq!(
        tally.artifact_refs(),
        ["seed.md".to_string(), "a/1.md".into(), "b/2.md".into()]
    );
    assert_eq!(sorted_decls(&tally), strs(&["notes", "report", "seeded"]));
}

/// Two tasks may each satisfy the same declaration — a resumed one may even
/// re-state a declaration the checkpoint already carried. The step only ever
/// asks whether *some* task did, so a declaration must count once however
/// many tasks claim it, or the checkpoint payload grows a duplicate on every
/// attempt.
#[test]
fn a_declaration_two_tasks_satisfy_is_counted_once() {
    let mut tally = StepTally::resuming(Some(&payload(&[], &["report"])));
    tally.fold(contribution(&[], &["report", "notes"]));
    tally.fold(contribution(&[], &["report"]));

    assert_eq!(sorted_decls(&tally), strs(&["notes", "report"]));
}

/// The per-task checkpoint payload is written alongside the task's own id,
/// so it must name that task's output and nothing else. Widening it to the
/// step's totals is what would attribute an earlier task's artifacts to a
/// later one on resume.
#[test]
fn a_contributions_payload_names_only_that_tasks_output() {
    let mut tally = StepTally::resuming(None);
    tally.fold(contribution(&["a/1.md"], &["notes"]));

    let second = contribution(&["b/2.md"], &["report"]);
    let produced = second.produced();
    tally.fold(second);

    assert_eq!(produced.artifact_refs, strs(&["b/2.md"]));
    assert_eq!(produced.satisfied_decls, strs(&["report"]));
}

/// Landed tasks are the resume prefix: the last one's SHA is the commit a
/// mid-list failure resets the worktree to before merging. Landed order is
/// therefore commit order, and reordering them would anchor the prefix at
/// the wrong commit.
#[test]
fn landed_tasks_stay_in_the_order_they_committed() {
    let mut tally = StepTally::resuming(None);
    tally.land(LandedTask {
        id: "t-1".into(),
        sha: "aaa".into(),
    });
    tally.land(LandedTask {
        id: "t-2".into(),
        sha: "bbb".into(),
    });

    let ids: Vec<&str> = tally.landed().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["t-1", "t-2"]);
    assert_eq!(tally.landed().last().map(|t| t.sha.as_str()), Some("bbb"));
}

/// The pre-V36 sweep recovers references the tally never earned, so they
/// have to reach downstream alongside the ones it did. Dropping them is what
/// leaves a resumed step carrying only its diff.
#[test]
fn recovered_references_join_the_ones_tasks_earned() {
    let mut tally = StepTally::resuming(None);
    tally.fold(contribution(&["a/1.md"], &[]));
    tally.recover_refs(strs(&["swept.md"]));

    assert_eq!(
        tally.artifact_refs(),
        ["a/1.md".to_string(), "swept.md".into()]
    );
}
