//! Everything the finalize agent is given to work from.
//!
//! The finalize agent runs with **no shell** (`StepCapability::ReadOnly`
//! denies `Bash`), which is what makes it structurally unable to open the
//! PR itself. The cost of that guarantee is that it cannot run `git log`
//! or `git diff` for itself — so Demeteo runs them and hands the results
//! over. This module is that hand-off.

use crate::paths;
use crate::ports::execution::ExecutionPort;

/// Hard cap on the diff we paste into the prompt. Big enough for a real
/// feature, small enough that a vendored-lockfile-sized change can't blow
/// the context window (and with it the step) on the very last hop of a
/// long, expensive run.
const MAX_DIFF_BYTES: usize = 60_000;

/// Commit subjects Demeteo itself wrote. They describe the machinery, not
/// the work, and the whole point of the squash is to make them disappear —
/// so they are also worthless as input for summarising the work.
fn is_plumbing_commit(subject: &str, feature_id: &str) -> bool {
    subject.starts_with("chore: merge subtask ")
        || subject.starts_with("chore: resolve ")
        || subject.starts_with(&format!("feat({}):", feature_id))
}

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
}

pub(crate) async fn gather_branch_work(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    repo_dir: &str,
    feature_branch: &str,
    default_branch: &str,
    feature_id: &str,
) -> BranchWork {
    let safe_dir = paths::shell_escape_posix(repo_dir);
    let git = |args: String| format!("git -C {} {}", safe_dir, args);
    let run = |cmd: String| async move {
        exec.run_command(machine_str, &cmd)
            .await
            .unwrap_or_default()
    };

    // Diff against the pushed default branch when we have it — that is what
    // the PR itself will be diffed against.
    let base_ref = if exec
        .run_command(
            machine_str,
            &git(format!(
                "rev-parse --verify -q refs/remotes/origin/{}",
                paths::shell_escape_posix(default_branch)
            )),
        )
        .await
        .is_ok()
    {
        format!("origin/{}", default_branch)
    } else {
        default_branch.to_string()
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
    let commit_log = raw_log
        .split('\u{1e}')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter(|entry| {
            let subject = entry.lines().next().unwrap_or("");
            !is_plumbing_commit(subject, feature_id)
        })
        .map(|entry| format!("- {}", entry.replace('\n', "\n  ")))
        .collect::<Vec<_>>()
        .join("\n");

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
    }
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
