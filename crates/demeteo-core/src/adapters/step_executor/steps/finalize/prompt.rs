//! The finalize agent's prompt.
//!
//! Two prompts: one to author the summary, one to repair a message the
//! repo's `commit-msg` hook rejected.

use super::context::BranchWork;

/// The wire contract, stated once and reused by both prompts.
const CONTRACT: &str = "\
Respond with ONLY a JSON object, and no other text:

{
  \"commit_subject\": \"the subject line of the single squashed commit\",
  \"commit_body\": \"the body of that commit: why the change was made, not a file-by-file list\",
  \"pr_title\": \"the pull request title\",
  \"pr_body\": \"the pull request description, in markdown\"
}

You have no shell and no network — Demeteo squashes the branch and opens the \
pull request itself, from the JSON you return. Do not attempt to run `git`, \
`gh`, `glab`, or `curl`; those tools are not available to you, and the PR is \
not yours to open. Your entire job is to write those four strings.";

/// Ask the agent to summarise the branch's work.
pub(crate) fn build_authoring_prompt(
    feature_title: &str,
    feature_description: &str,
    feature_branch: &str,
    base_branch: &str,
    work: &BranchWork,
) -> String {
    let truncation_note = if work.diff_truncated {
        "\n(The diff below is truncated — summarise from the reports and commit log above \
         plus what is shown.)"
    } else {
        ""
    };

    // Best-effort enrichment: the spec / review / check reports from earlier
    // steps carry the *intent* the raw diff can't. Omitted entirely when the
    // workflow produced none, so the prompt degrades to diff-only.
    let prior_work_section = if work.prior_work.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n## Reports from earlier steps of this run\n\
             These are the spec, review, and checks for this work. Use them for the *why* \
             and the approach; trust the diff below for what actually shipped.\n{}\n",
            work.prior_work.trim()
        )
    };

    format!(
        "You are writing the permanent record of a piece of work: the single commit \
         that will land on `{base_branch}`, and the pull request a reviewer will read \
         before approving it.

Every commit on `{feature_branch}` is about to be collapsed into ONE commit. The \
step-by-step history (and Demeteo's own bookkeeping commits) will be gone — your \
message is what survives. Write it for the person who runs `git log` in six months \
and needs to know why this change exists.

## What was asked for
{feature_title}

{feature_description}
{prior_work_section}
## The commits on the branch (Demeteo's own bookkeeping commits already removed)
{commit_log}

## Files changed
{diff_stat}

## How this repo writes commits
Match it. If it uses Conventional Commits, use the same types and scopes it actually \
uses — do not invent a scope that does not appear below. If a commitlint config is \
shown, your subject MUST satisfy it (including any subject-length limit).
{conventions}

## The diff{truncation_note}
```diff
{diff}
```

Write the commit subject in the imperative mood, describing the change, not the \
process (\"add retry budget to the harness gate\", never \"as requested, I have \
added…\"). The commit body and the PR body should explain the motivation and the \
approach, and call out anything a reviewer should look at closely. Do not pad them \
with a file-by-file walkthrough — the diff is right there.

{CONTRACT}",
        base_branch = base_branch,
        feature_branch = feature_branch,
        feature_title = feature_title,
        feature_description = feature_description,
        prior_work_section = prior_work_section,
        commit_log = if work.commit_log.trim().is_empty() {
            "(no commits with a human-written message — work from the diff)"
        } else {
            &work.commit_log
        },
        diff_stat = work.diff_stat.trim(),
        conventions = work.conventions.trim(),
        truncation_note = truncation_note,
        diff = work.diff.trim(),
        CONTRACT = CONTRACT,
    )
}

/// The repo's own `commit-msg` hook rejected the proposed message. Hand the
/// agent the hook's verdict and let it repair the message.
///
/// This is the whole reason the hook is run as a *validator* before the
/// commit exists: a rejection becomes one more turn of a loop that converges,
/// instead of a failed commit that leaves the pipeline stuck.
pub(crate) fn build_repair_prompt(rejected_subject: &str, hook_output: &str) -> String {
    format!(
        "This repository's `commit-msg` hook REJECTED that commit message.

The subject you proposed was:
    {rejected_subject}

The hook said:
```
{hook_output}
```

Fix the message so it satisfies the hook, keeping the same meaning. Read the \
hook's complaints literally — if it wants a type prefix, add the type it lists; \
if it wants a shorter subject, shorten it; if it wants a scope from a fixed set, \
use one from that set.

{CONTRACT}",
        rejected_subject = rejected_subject,
        hook_output = hook_output.trim(),
        CONTRACT = CONTRACT,
    )
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/finalize/prompt.rs"]
mod tests;
