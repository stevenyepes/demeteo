//! Everything the finalize agent is given to work from.
//!
//! The finalize agent runs with **no shell** (`StepCapability::ReadOnly`
//! denies `Bash`), which is what makes it structurally unable to open the
//! PR itself. The cost of that guarantee is that it cannot run `git log`
//! or `git diff` for itself — so Demeteo runs them and hands the results
//! over. This module is that hand-off.

use super::RepoSite;
use crate::domain::finalize::commit_log::real_commit_log;
use crate::domain::models::StepExecution;
use crate::paths;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::execution::ExecutionPort;

/// Hard cap on the diff we inline into the prompt.
///
/// Deliberately small. On a remote (Desktop→SSH) run the prompt is passed as
/// the trailing argv of `claude`, and the whole shell command is sent as a
/// *single* SSH channel `exec` request — libssh2 rejects an oversized request
/// with `-34` ("unable to send channel request"), which is why a big-diff
/// feature's finalize step failed to spawn its agent at all. The diff is now
/// a bounded *reality-check* excerpt: the prose reports from earlier steps
/// (see [`gather_prior_artifacts`]) carry the intent, and `diff --stat`
/// carries the full file list.
const MAX_DIFF_BYTES: usize = 15_000;

/// Per-artifact and overall caps for the best-effort prior-step reports.
/// Bounded for the same SSH-exec-size reason as the diff.
const MAX_ARTIFACT_BYTES: usize = 4_000;
const MAX_PRIOR_WORK_BYTES: usize = 12_000;

/// The material the agent summarises.
pub(crate) struct BranchWork {
    /// Real commit subjects+bodies on the branch, plumbing filtered out.
    pub commit_log: String,
    /// `git diff --stat` against the merge base.
    pub diff_stat: String,
    /// The diff itself, truncated to [`MAX_DIFF_BYTES`].
    pub diff: String,
    pub diff_truncated: bool,
    /// Whatever the repo says about how commits should be written:
    /// commitlint config, CONTRIBUTING, and the observed house style.
    pub conventions: String,
    /// Best-effort prose reports from earlier steps (spec, validation,
    /// critic, …). Empty when the workflow declared none or the artifacts
    /// weren't captured — finalize enriches with these but never depends on
    /// them. See [`gather_prior_artifacts`].
    pub prior_work: String,
}

/// The two ends of the range finalize summarises. `base_branch` is what the
/// run declared itself measured against
/// ([`diff_base::resolve`](crate::domain::diff_base::resolve)), not the
/// project's default branch: a run based on anything else would otherwise be
/// summarised against a range holding every commit its base is missing — the
/// agent writing the PR title and body from a diff that is not the PR's.
pub(crate) struct BranchRange<'a> {
    pub feature_branch: &'a str,
    pub base_branch: &'a str,
}

/// What earlier steps left behind, and where to read it from.
pub(crate) struct PriorWork<'a> {
    pub artifacts: &'a dyn ArtifactStore,
    pub steps: &'a [StepExecution],
    pub finalize_step_id: &'a str,
}

pub(crate) async fn gather_branch_work(
    exec: &dyn ExecutionPort,
    site: RepoSite<'_>,
    range: BranchRange<'_>,
    prior: PriorWork<'_>,
    feature_id: &str,
) -> BranchWork {
    let RepoSite {
        machine: machine_str,
        repo_dir,
    } = site;
    let BranchRange {
        feature_branch,
        base_branch,
    } = range;
    let safe_dir = paths::shell_escape_posix(repo_dir);
    let git = |args: String| format!("git -C {} {}", safe_dir, args);
    let run = |cmd: String| async move {
        exec.run_command(machine_str, &cmd)
            .await
            .unwrap_or_default()
    };

    // Diff against the pushed base branch when we have it — that is what
    // the PR itself will be diffed against.
    let base_ref = if exec
        .run_command(
            machine_str,
            &git(format!(
                "rev-parse --verify -q refs/remotes/origin/{}",
                paths::shell_escape_posix(base_branch)
            )),
        )
        .await
        .is_ok()
    {
        format!("origin/{}", base_branch)
    } else {
        base_branch.to_string()
    };
    let safe_base = paths::shell_escape_posix(&base_ref);
    let safe_fb = paths::shell_escape_posix(feature_branch);
    let range = format!("{}..{}", safe_base, safe_fb);

    // `%s%n%b` per commit, `%x1e` between them, so a multi-line body can't
    // be confused for the next commit's subject.
    let raw_log = run(git(format!(
        "log --no-merges --format='%s%n%b%x1e' {}",
        range
    )))
    .await;
    let commit_log = real_commit_log(&raw_log, feature_id);

    let diff_stat = run(git(format!("diff --stat {}", range))).await;

    let full_diff = run(git(format!("diff {}", range))).await;
    let diff_truncated = full_diff.len() > MAX_DIFF_BYTES;
    let diff = if diff_truncated {
        // Keep the head: a diff's first hunks are the ones that identify
        // what the change *is*, which is what we're asking the agent for.
        let mut cut = MAX_DIFF_BYTES;
        while cut > 0 && !full_diff.is_char_boundary(cut) {
            cut -= 1;
        }
        full_diff[..cut].to_string()
    } else {
        full_diff
    };

    let conventions = gather_conventions(exec, machine_str, repo_dir, &safe_dir, &base_ref).await;

    BranchWork {
        commit_log,
        diff_stat,
        diff,
        diff_truncated,
        conventions,
        prior_work: gather_prior_artifacts(prior.artifacts, prior.steps, prior.finalize_step_id),
    }
}

/// Best-effort context from earlier steps: the prose reports they produced
/// (spec, validation, critic, …), read from the artifact store the driver is
/// wired with.
///
/// Universal across local / desktop-SSH / detached-runner runs because the
/// store is always local to the driver process (both the desktop and the
/// runner build it through the same `build_core_context`). Best-effort by
/// design: a workflow may declare no report-producing steps, and remote
/// artifact capture can silently produce nothing — so this returns whatever
/// exists and an empty string when nothing does. Raw diffs/patches are
/// skipped: the diff is already inlined separately, bounded. The finalize
/// step itself is skipped (it hasn't produced anything yet).
pub(crate) fn gather_prior_artifacts(
    artifacts: &dyn ArtifactStore,
    steps: &[StepExecution],
    finalize_step_id: &str,
) -> String {
    let mut out = String::new();
    for step in steps {
        if step.step_id.0 == finalize_step_id {
            continue;
        }
        for path in &step.artifact_paths {
            if path.ends_with(".diff") || path.ends_with(".patch") {
                continue;
            }
            let Ok(body) = artifacts.get(path) else {
                continue;
            };
            let body = body.trim();
            if body.is_empty() {
                continue;
            }
            let name = path.rsplit('/').next().unwrap_or(path.as_str());
            let mut chunk = body.to_string();
            if chunk.len() > MAX_ARTIFACT_BYTES {
                let mut cut = MAX_ARTIFACT_BYTES;
                while cut > 0 && !chunk.is_char_boundary(cut) {
                    cut -= 1;
                }
                chunk.truncate(cut);
                chunk.push_str("\n… (truncated)");
            }
            out.push_str(&format!(
                "\n--- {} (from step `{}`) ---\n{}\n",
                name, step.step_id.0, chunk
            ));
            if out.len() >= MAX_PRIOR_WORK_BYTES {
                return out;
            }
        }
    }
    out
}

/// What this repo considers a well-formed commit — read from whatever it
/// actually ships, rather than assumed. A repo with a commitlint config
/// gets its rules honoured; a repo with none gets its house style inferred
/// from the commits already on the default branch.
async fn gather_conventions(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    repo_dir: &str,
    safe_dir: &str,
    base_ref: &str,
) -> String {
    const CONVENTION_FILES: &[&str] = &[
        "commitlint.config.js",
        "commitlint.config.cjs",
        "commitlint.config.mjs",
        "commitlint.config.ts",
        ".commitlintrc",
        ".commitlintrc.json",
        ".commitlintrc.js",
        ".commitlintrc.yml",
        ".gitmessage",
        "CONTRIBUTING.md",
    ];
    /// A CONTRIBUTING.md can be book-length; we only want its conventions.
    const MAX_CONVENTION_BYTES: usize = 4_000;

    let mut out = String::new();
    for name in CONVENTION_FILES {
        let path = format!("{}/{}", repo_dir.trim_end_matches('/'), name);
        let Ok(content) = exec.read_file(machine_str, &path).await else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        let mut body = content;
        if body.len() > MAX_CONVENTION_BYTES {
            let mut cut = MAX_CONVENTION_BYTES;
            while cut > 0 && !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
            body.push_str("\n… (truncated)");
        }
        out.push_str(&format!("\n--- {} ---\n{}\n", name, body.trim()));
    }

    // The house style, as practised rather than as documented.
    if let Ok(subjects) = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} log --format=%s -20 {}",
                safe_dir,
                paths::shell_escape_posix(base_ref)
            ),
        )
        .await
    {
        if !subjects.trim().is_empty() {
            out.push_str(&format!(
                "\n--- the last 20 commit subjects on {} ---\n{}\n",
                base_ref,
                subjects.trim()
            ));
        }
    }

    if out.trim().is_empty() {
        "(this repo ships no commit convention config; use Conventional Commits)".to_string()
    } else {
        out
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/finalize/context.rs"]
mod tests;
