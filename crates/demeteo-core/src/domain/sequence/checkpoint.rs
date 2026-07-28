//! What a previous attempt's landed prefix means for this one.
//!
//! The whole of the crash-resume decision, as a synchronous function over
//! what the adapter observed. The adapter asks git two questions and
//! reports the answers as an [`AnchorProbe`]; [`classify`] turns the
//! checkpoint row plus that probe into a [`CheckpointResume`], and nothing
//! in here can await, retry, or ask git anything itself.
//!
//! That split is the point. The rules below decide whether a fresh
//! worktree gets `reset --hard`ed backwards onto an old commit, and the
//! cost of getting one wrong is asymmetric: a wrong `None` re-runs tasks
//! that were already paid for, while a wrong `Restore` destroys merged
//! work. Twelve input combinations decide it. While they lived inside an
//! `async fn` that also ran `git cat-file` and `git merge-base`, reaching
//! them from a test meant building an `ExecutionDriver` and its twenty-odd
//! port doubles — so three of the twelve were covered, through a helper,
//! and the rest were argued for in comments.

use crate::domain::models::{CheckpointProduced, SequenceCheckpoint};
use crate::domain::sequence::sha::Sha;

/// What git said about the checkpoint's anchor commit.
///
/// Deliberately four states, not a `bool` and not a `Result`. "Git could
/// not answer" is a distinct verdict from "not merged", and collapsing
/// them is the specific mistake that would reset a worktree backwards past
/// work that *was* merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorProbe {
    /// The anchor commit is not in the repo: the ref was deleted, or the
    /// repo was replaced under us. There is nothing to resume onto.
    Missing,
    /// The anchor is contained in the feature branch — the merge already
    /// happened, and a freshly-cut worktree carries the prefix.
    Merged,
    /// The anchor is off on the step branch alone. The signature of an
    /// attempt interrupted between committing a task and merging its
    /// prefix.
    Stranded,
    /// Git never answered: unreachable machine, timeout, corrupt object,
    /// unrelated histories. **Not** "not merged" — see the type's note.
    Unknown,
}

/// Read a `git merge-base <anchor> <base>` result as a verdict on whether
/// the anchor is already contained in the base.
///
/// The question is deliberately asked in a form whose answer arrives on
/// **stdout**. `merge-base --is-ancestor` puts its verdict in the exit code
/// — `0` yes, `1` no, `128` "git could not answer" — and `ExecutionPort`
/// flattens every non-zero exit into the same bare-stderr `Err`, so a
/// corrupt object or a vanished ref would be indistinguishable from a
/// confident "no" and would send the caller down the arm that resets a
/// worktree backwards. Asking for the merge base itself removes the class:
/// git prints it and exits 0, or it fails and we know nothing. Every `Err`,
/// transport or otherwise, is then honestly "unknown" — no error-string
/// classification required.
pub fn anchor_is_merged(merge_base_stdout: &str, anchor: &Sha) -> bool {
    let base = merge_base_stdout.trim();
    !base.is_empty() && base.eq_ignore_ascii_case(anchor.as_str().trim())
}

/// Decide what a previous attempt's landed prefix means for this one.
///
/// The two recovery modes are told apart by what git said, not by
/// remembering which code path wrote the row — the row itself cannot say,
/// since the mid-list failure path and the task loop both write one and
/// only the former merges. The repo is the thing that actually knows.
///
/// **Every uncertainty resolves to [`CheckpointResume::None`].** A full
/// re-run wastes money; a wrong skip loses work. `probe` is only consulted
/// for a row that carries an anchor, so a caller that has no anchor to
/// probe need not invent an answer.
pub fn classify(checkpoint: SequenceCheckpoint, probe: AnchorProbe) -> CheckpointResume {
    if checkpoint.is_empty() {
        return CheckpointResume::None;
    }

    let SequenceCheckpoint {
        landed_task_ids,
        anchor_sha,
        produced,
    } = checkpoint;

    // A row with no anchor predates V35, and only one writer existed
    // then — the one that merges the prefix before recording it.
    let Some(anchor) = anchor_sha else {
        return CheckpointResume::Merged {
            landed_ids: landed_task_ids,
            produced,
        };
    };

    match probe {
        AnchorProbe::Merged => CheckpointResume::Merged {
            landed_ids: landed_task_ids,
            produced,
        },
        AnchorProbe::Stranded => CheckpointResume::Restore {
            landed_ids: landed_task_ids,
            sha: Sha::new(anchor),
            produced,
        },
        AnchorProbe::Missing | AnchorProbe::Unknown => CheckpointResume::None,
    }
}

/// What this attempt should do about the tasks a previous one finished.
///
/// Resolved *before* the task plan, because the two decisions are one
/// decision: dropping a task from the plan is only safe if its work will
/// actually be in the worktree the remaining tasks open. Deciding them
/// apart is how a resume ends up implementing task 21 on top of a hole
/// where tasks 1-20 should be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointResume {
    /// No checkpoint, or one that can no longer be trusted (its anchor
    /// commit is gone, or a probe failed and we cannot tell). Run the full
    /// plan — the pre-V32 behaviour, and the only safe default.
    None,
    /// The prefix is already on the feature branch, so the freshly-cut
    /// worktree carries it. Skip the ids and touch nothing. This is what
    /// every V32-era checkpoint meant, and what the mid-list failure path
    /// still produces.
    Merged {
        landed_ids: Vec<String>,
        produced: Option<CheckpointProduced>,
    },
    /// The prefix is committed on the step branch only — the signature of
    /// an *interrupted* attempt, which never reached the merge. Skip the
    /// ids, but move the worktree onto `sha` first.
    Restore {
        landed_ids: Vec<String>,
        sha: Sha,
        produced: Option<CheckpointProduced>,
    },
}

impl CheckpointResume {
    /// The tasks this attempt must not re-run. Empty for [`Self::None`].
    pub fn landed_ids(&self) -> &[String] {
        match self {
            Self::None => &[],
            Self::Merged { landed_ids, .. } | Self::Restore { landed_ids, .. } => landed_ids,
        }
    }

    /// What those tasks emitted, or `None` when the row cannot say — a
    /// pre-V36 checkpoint, or one whose payload would not parse.
    ///
    /// `None` means **unknown**, and callers must not read it as "they
    /// produced nothing": the difference decides whether a step whose
    /// deliverable is already on disk passes its declared-artifact check
    /// or is failed for a deliverable it did produce.
    pub fn produced(&self) -> Option<&CheckpointProduced> {
        match self {
            Self::None => Option::None,
            Self::Merged { produced, .. } | Self::Restore { produced, .. } => produced.as_ref(),
        }
    }

    /// The checkpoint row this resume was read from — what the row has to
    /// be put back to when the attempt that grew it is discarded.
    ///
    /// [`Self::Merged`] rewinds to an anchor-less row on purpose: the
    /// prefix is on the feature branch, so "skip these ids, touch nothing"
    /// is the whole instruction, and an anchor would only invite a restore
    /// nobody needs. The next `resolve_checkpoint_resume` reads that row
    /// back as `Merged`, so the rewind is idempotent.
    pub fn as_stored(&self) -> (&[String], Option<&Sha>, Option<&CheckpointProduced>) {
        match self {
            Self::None => (&[], Option::None, Option::None),
            Self::Merged {
                landed_ids,
                produced,
            } => (landed_ids, Option::None, produced.as_ref()),
            Self::Restore {
                landed_ids,
                sha,
                produced,
            } => (landed_ids, Some(sha), produced.as_ref()),
        }
    }
}

/// What a rollback should do to the landed checkpoint.
///
/// Every caller of [`cleanup_and_rollback`](crate::adapters::step_executor::driver::ExecutionDriver) is discarding
/// this attempt, so the default is [`Self::RewindTo`]: the checkpoint moves
/// back with the branch, to exactly the row this attempt started from. The
/// one exception states itself.
pub enum CheckpointDisposition<'a> {
    /// Put the checkpoint back to the state this attempt found it in.
    RewindTo(&'a CheckpointResume),
    /// Leave this attempt's checkpoint standing.
    ///
    /// Only for the mid-list failure whose prefix *merge* failed: the tasks
    /// finished and their commits are pinned, so the next attempt restoring
    /// and resuming from them beats re-running and re-paying for them. The
    /// rollback here is about the feature branch, not about disowning the
    /// work.
    Keep,
    /// Drop the checkpoint outright: clear the row and unpin the prefix.
    ///
    /// For the one failure [`Self::RewindTo`] cannot terminate. When every
    /// task was already checkpointed, the attempt runs none of them and
    /// goes straight to its tail — and if the *step's own verdict* then
    /// rejects it (the verifier, a missing declared deliverable, an empty
    /// diff), rewinding puts the row back to exactly the state that
    /// produced the rejection. The retry reads it, again runs zero tasks,
    /// again reaches the same verdict on the same tree: the budget is
    /// spent on identical verifier passes, no agent ever sees the
    /// feedback, and no code changes. Rewinding is right when the work is
    /// sound and the *attempt* failed; here the work is what was judged,
    /// so the claim on it has to go.
    ///
    /// Deliberately **not** used for an environmental failure on the same
    /// path — a merge that could not run says nothing about the landed
    /// work, and discarding there would re-run a list that is fine.
    Discard,
}

/// What to do with the checkpoint when the step fails on **its own verdict**
/// — the verifier rejected the work, a declared deliverable is missing, the
/// branch carries no changes.
///
/// The ordinary answer is to rewind: the attempt is discarded, and the row
/// goes back to what it was before it ran. That answer is wrong in exactly
/// one case, and the parameter is the whole condition. When every task was
/// already checkpointed, the attempt ran none of them, so the thing the
/// verdict just rejected *is* the checkpoint — rewinding restores it, the
/// retry skips every task again, and reaches the same verdict on the same
/// tree until the budget runs out. Discarding is what lets the next attempt
/// re-implement with the feedback in hand.
///
/// Callers on environmental failures must not use this: an unreachable
/// machine or a failed merge is not a judgement on the work.
pub fn verdict_disposition(
    resumed_whole_list: bool,
    resume: &CheckpointResume,
) -> CheckpointDisposition<'_> {
    if resumed_whole_list {
        CheckpointDisposition::Discard
    } else {
        CheckpointDisposition::RewindTo(resume)
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/sequence/checkpoint.rs"]
mod tests;
