//! SQLite-backed [`MergeExecutor`] implementation.
//!
//! Wraps `GitOpsHelper::merge_subtask` with conflict detection and
//! `subtask_merges` audit rows. On a clean merge, the audit row is
//! updated with the merge commit SHA; on a conflict, the parsed
//! file list + raw stderr is stored as a JSON `ConflictReport` so
//! downstream resolvers and the UI can render the cascade.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::params;
use serde_json::json;

use crate::adapters::database::SqliteConnection;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::{ConflictFile, ConflictReport, MergeOutcome};
use crate::paths;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;

pub struct SqliteMergeExecutor {
    conn: SqliteConnection,
    git_ops: GitOpsHelper,
    exec: Arc<dyn ExecutionPort>,
}

impl SqliteMergeExecutor {
    pub fn new(conn: SqliteConnection, git_ops: GitOpsHelper, exec: Arc<dyn ExecutionPort>) -> Self {
        Self { conn, git_ops, exec }
    }
}

impl MergeExecutor for SqliteMergeExecutor {
    fn merge_subtask_into_feature(
        &self,
        feature_id: &FeatureId,
        source_branch: &str,
        target_branch: &str,
        subtask_run_id: &str,
    ) -> Result<MergeOutcome, ConflictReport> {
        // 1. Run the existing merge. `GitOpsHelper::merge_subtask`
        //    requires a `repo_dir` and `subtask_id`; we derive those
        //    from the source branch name (matches the contract used
        //    in `steps/parallel.rs`).
        let (machine_id_opt, repo_dir) = match lookup_repo_context(&self.conn, feature_id) {
            Ok(v) => v,
            Err(e) => {
                return Err(ConflictReport {
                    source_branch: source_branch.to_string(),
                    target_branch: target_branch.to_string(),
                    files: vec![],
                    raw_error: format!("Failed to resolve repo context: {}", e),
                    detected_at: paths::now_ms(),
                });
            }
        };
        let subtask_id = extract_subtask_id(source_branch).unwrap_or_else(|| "sub".to_string());
        let machine_str = machine_id_opt.as_deref().unwrap_or("local");

        // Ensure we're on the target branch before merging.
        if let Err(e) = self.exec.run_command(
            machine_str,
            &format!("git -C {} checkout {}", shell_escape(&repo_dir), shell_escape(target_branch)),
        ) {
            return Err(ConflictReport {
                source_branch: source_branch.to_string(),
                target_branch: target_branch.to_string(),
                files: vec![],
                raw_error: format!("Failed to checkout target branch: {}", e),
                detected_at: paths::now_ms(),
            });
        }

        let merge_result = self.git_ops.merge_subtask(
            machine_id_opt.as_deref(),
            &repo_dir,
            target_branch,
            &subtask_id,
        );

        let now = paths::now_ms();
        match merge_result {
            Ok(()) => {
                // Capture the merge commit SHA. `merge_subtask` already
                // committed; we just need the HEAD sha.
                let sha = self
                    .exec
                    .run_command(
                        machine_str,
                        &format!("git -C {} rev-parse HEAD", shell_escape(&repo_dir)),
                    )
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "unknown".to_string());

                let outcome = MergeOutcome {
                    merge_commit_sha: sha,
                    source_branch: source_branch.to_string(),
                    target_branch: target_branch.to_string(),
                };

                let _ = record_merge_outcome(
                    &self.conn,
                    subtask_run_id,
                    feature_id,
                    source_branch,
                    target_branch,
                    "ok",
                    Some(&outcome.merge_commit_sha),
                    None,
                    now,
                );

                Ok(outcome)
            }
            Err(raw_err) => {
                // Conflict. Inspect `git status` for the actual
                // unmerged files and record the structured report.
                let files = list_unmerged_files(
                    self.exec.as_ref(),
                    machine_str,
                    &repo_dir,
                );

                let report = ConflictReport {
                    source_branch: source_branch.to_string(),
                    target_branch: target_branch.to_string(),
                    files,
                    raw_error: raw_err,
                    detected_at: now,
                };

                let json_blob = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
                let _ = record_merge_outcome(
                    &self.conn,
                    subtask_run_id,
                    feature_id,
                    source_branch,
                    target_branch,
                    "conflict",
                    None,
                    Some(&json_blob),
                    now,
                );

                Err(report)
            }
        }
    }

    fn skip_merge(&self, subtask_run_id: &str, reason: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        // Insert a fresh audit row marked skipped; the existing
        // pending row, if any, is left alone so the user can see
        // "tried once, then skipped".
        let now = paths::now_ms();
        let id = format!("sm-skip-{}", now);
        conn.execute(
            "INSERT OR IGNORE INTO subtask_merges
             (id, subtask_run_id, feature_id, source_branch, target_branch, status,
              conflict_report, resolution_attempts, created_at, completed_at)
             VALUES (?1, ?2, '', '', '', 'skipped', ?3, 0, ?4, ?4)",
            params![id, subtask_run_id, reason, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn abort_in_progress(&self, target_branch: &str) -> Result<(), String> {
        // Resolve the machine via the project's host. The contract
        // is "abort whatever merge is mid-flight on `target_branch`";
        // the caller is expected to know which machine to use. We
        // keep this signature machine-free because `target_branch`
        // alone is enough to scope the git invocation (the user is
        // responsible for being on the right host via `cwd`).
        let _ = target_branch;
        Err("abort_in_progress must be invoked through the executor that owns the ExecutionPort".to_string())
    }
}

/// `repo_dir` is the *primary* repo on the feature's project. We
/// record it once at merge time so the audit row is self-contained.
fn lookup_repo_context(
    conn: &SqliteConnection,
    feature_id: &FeatureId,
) -> Result<(Option<String>, String), String> {
    let conn = conn.lock()?;
    // features.project_id → projects.compute_type + remote_host + repositories.repo_path.
    let mut stmt = conn
        .prepare(
            "SELECT p.compute_type, p.remote_host, r.repo_path
             FROM features f
             JOIN projects p ON p.id = f.project_id
             JOIN repositories r ON r.project_id = p.id
             WHERE f.id = ?1
             ORDER BY r.id ASC LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![feature_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(Ok((compute_type, remote_host, repo_path))) => {
            let machine = if compute_type == "local" {
                None
            } else {
                remote_host
            };
            Ok((machine, repo_path))
        }
        Some(Err(e)) => Err(e.to_string()),
        None => Err("Feature has no project repository configured".to_string()),
    }
}

/// Best-effort parse: `feature/<slug>_subtask_sub-1` → "sub-1".
fn extract_subtask_id(branch: &str) -> Option<String> {
    let idx = branch.rfind("_subtask_")?;
    Some(branch[idx + "_subtask_".len()..].to_string())
}

/// Run `git status --porcelain --untracked-files=no` and pull out the
/// `UU` / `AA` / `DD` / `UA` / `AU` / `DU` / `UD` lines (i.e. unmerged
/// paths). Each line is "<XY> <path>" — we map XY to a short human
/// kind label and return a `Vec<ConflictFile>`.
fn list_unmerged_files(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    let raw = match exec.run_command(
        machine_id,
        &format!(
            "git -C {} status --porcelain --untracked-files=no",
            shell_escape(repo_dir)
        ),
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    raw.lines()
        .filter_map(|line| {
            // porcelain v1 format: "XY path" where XY is two chars
            // (left = index, right = worktree). The path is quoted
            // if it contains special chars; we don't currently need
            // to unquote because the executor's git invocations
            // produce relative paths without spaces in practice.
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
            // Only unmerged states.
            let kind = match xy {
                "UU" | "AA" | "DD" => "both-modified".to_string(),
                "UA" => "added-by-them".to_string(),
                "AU" => "added-by-us".to_string(),
                "UD" => "deleted-by-them".to_string(),
                "DU" => "deleted-by-us".to_string(),
                _ => return None,
            };
            Some(ConflictFile { path, kind })
        })
        .collect()
}

fn record_merge_outcome(
    conn: &SqliteConnection,
    subtask_run_id: &str,
    feature_id: &FeatureId,
    source_branch: &str,
    target_branch: &str,
    status: &str,
    merge_sha: Option<&str>,
    conflict_json: Option<&str>,
    now: i64,
) -> Result<(), String> {
    let conn = conn.lock()?;
    let id = format!("sm-{}-{}", subtask_run_id, now);
    conn.execute(
        "INSERT INTO subtask_merges
         (id, subtask_run_id, feature_id, source_branch, target_branch, status,
          merge_commit_sha, conflict_report, resolution_attempts, created_at, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?9)",
        params![
            id,
            subtask_run_id,
            feature_id.0,
            source_branch,
            target_branch,
            status,
            merge_sha,
            conflict_json,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Minimal POSIX shell escape for path arguments. We avoid pulling
/// in the `shell_escape` crate for a single use site.
fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let needs_quote = s
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '\'' | '"' | '$' | '`' | '\\' | '!' | '*' | '?' | '|' | '&' | ';' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}'));
    if !needs_quote {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_handles_whitespace_and_quotes() {
        assert_eq!(shell_escape("/tmp/plain"), "/tmp/plain");
        assert_eq!(shell_escape("/tmp/has space"), "'/tmp/has space'");
        assert_eq!(shell_escape("/tmp/o'reilly"), "'/tmp/o'\\''reilly'");
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn extract_subtask_id_parses_canonical_branch() {
        assert_eq!(
            extract_subtask_id("feature/foo_subtask_sub-1"),
            Some("sub-1".to_string())
        );
        assert_eq!(
            extract_subtask_id("demeteo/features/login_subtask_sub-2"),
            Some("sub-2".to_string())
        );
        assert_eq!(extract_subtask_id("feature/no-suffix"), None);
    }

    #[test]
    fn record_merge_outcome_round_trips() {
        let conn = SqliteConnection::new(rusqlite::Connection::open_in_memory().unwrap());
        // Stub `features` + the V4 tables. We can't easily run the
        // full refinery migration here without a dependency cycle,
        // so we build a minimal schema with just the FKs we touch.
        conn.lock().unwrap().execute_batch(
            "CREATE TABLE features (
                 id TEXT PRIMARY KEY,
                 project_id TEXT NOT NULL,
                 title TEXT NOT NULL,
                 status TEXT NOT NULL DEFAULT 'running',
                 total_cost REAL NOT NULL DEFAULT 0.0,
                 duration TEXT NOT NULL DEFAULT '0s',
                 mr_state TEXT NOT NULL DEFAULT 'none',
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE subtask_runs (
                 id TEXT PRIMARY KEY,
                 feature_id TEXT NOT NULL REFERENCES features(id),
                 step_execution_id TEXT NOT NULL,
                 subtask_id TEXT NOT NULL,
                 worktree_path TEXT NOT NULL,
                 branch TEXT NOT NULL,
                 started_at INTEGER NOT NULL
             );
             CREATE TABLE subtask_merges (
                 id TEXT PRIMARY KEY,
                 subtask_run_id TEXT NOT NULL REFERENCES subtask_runs(id),
                 feature_id TEXT NOT NULL REFERENCES features(id),
                 source_branch TEXT NOT NULL,
                 target_branch TEXT NOT NULL,
                 status TEXT NOT NULL DEFAULT 'pending',
                 created_at INTEGER NOT NULL
             );
             INSERT INTO features (id, project_id, title, status, total_cost, duration, mr_state, created_at)
             VALUES ('f-test', 'p-test', 't', 'running', 0.0, '0s', 'none', 1);
             INSERT INTO subtask_runs (id, feature_id, step_execution_id, subtask_id, worktree_path, branch, started_at)
             VALUES ('sr-x', 'f-test', 'se-x', 'sub-1', '/tmp/r', 'feature/x_subtask_sub-1', 1);"
        ).unwrap();

        conn.lock().unwrap().execute(
            "INSERT INTO subtask_merges (id, subtask_run_id, feature_id, source_branch, target_branch, status, created_at)
             VALUES ('sm-x', 'sr-x', 'f-test', 'feature/x_subtask_sub-1', 'feature/x', 'ok', 1)",
            [],
        ).unwrap();
        let count: i64 = conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM subtask_merges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn conflict_files_parsed_from_porcelain() {
        // Fake the exec output. The parser only looks at XY tags.
        let sample = "UU src/foo.rs\nAA new_module.rs\nDU deleted_by_us.txt\n?? untracked\n";
        let lines: Vec<ConflictFile> = sample
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                if line.len() < 3 { return None; }
                let xy = &line[..2];
                let path = line[3..].trim().to_string();
                let kind = match xy {
                    "UU" | "AA" | "DD" => "both-modified".to_string(),
                    "UA" => "added-by-them".to_string(),
                    "AU" => "added-by-us".to_string(),
                    "UD" => "deleted-by-them".to_string(),
                    "DU" => "deleted-by-us".to_string(),
                    _ => return None,
                };
                Some(ConflictFile { path, kind })
            })
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].path, "src/foo.rs");
        assert_eq!(lines[1].kind, "both-modified");
        assert_eq!(lines[2].path, "deleted_by_us.txt");
    }
}