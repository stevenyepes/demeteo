use rusqlite::params;

use crate::domain::ids::FeatureId;
use crate::domain::models::{ConflictReport, RepoContext, WorktreeContext};
use crate::ports::db::MergeAuditRepository;

use super::super::SqliteAdapter;

impl MergeAuditRepository for SqliteAdapter {
    #[allow(clippy::too_many_arguments)]
    fn record_merge_outcome(
        &self,
        subtask_run_id: &str,
        feature_id: &FeatureId,
        source_branch: &str,
        target_branch: &str,
        status: &str,
        merge_sha: Option<&str>,
        conflict_json: Option<&str>,
        now: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
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

    #[allow(clippy::too_many_arguments)]
    fn record_sync_outcome(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        default_branch: &str,
        status: &str,
        merge_sha: Option<&str>,
        conflict_json: Option<&str>,
        now: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let id = format!("fs-{}-{}", feature_id.0, now);
        conn.execute(
            "INSERT INTO feature_syncs
             (id, feature_id, feature_branch, default_branch, status,
              merge_commit_sha, conflict_report, resolution_attempts, created_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?8)",
            params![
                id,
                feature_id.0,
                feature_branch,
                default_branch,
                status,
                merge_sha,
                conflict_json,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn lookup_worktree_context(
        &self,
        feature_id: &FeatureId,
        subtask_run_id: &str,
    ) -> Result<WorktreeContext, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT p.compute_type, p.remote_host, p.id, r.repo_path, sr.worktree_path
                 FROM features f
                 JOIN projects p ON p.id = f.project_id
                 JOIN repositories r ON r.project_id = p.id
                 JOIN subtask_runs sr ON sr.feature_id = f.id
                 WHERE f.id = ?1 AND sr.id = ?2
                 ORDER BY r.id ASC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![feature_id.0, subtask_run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok((compute_type, remote_host, project_id, repo_path, wt_path))) => {
                Ok(WorktreeContext {
                    compute_type,
                    remote_host,
                    project_id,
                    repo_path,
                    worktree_path: wt_path,
                })
            }
            Some(Err(e)) => Err(e.to_string()),
            None => Err("Feature has no project repository configured".to_string()),
        }
    }

    fn lookup_repo_context(&self, feature_id: &FeatureId) -> Result<RepoContext, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT p.compute_type, p.remote_host, p.id, r.repo_path
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
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok((compute_type, remote_host, project_id, repo_path))) => Ok(RepoContext {
                compute_type,
                remote_host,
                project_id,
                repo_path,
            }),
            Some(Err(e)) => Err(e.to_string()),
            None => Err("Feature has no project repository configured".to_string()),
        }
    }

    fn get_last_sync_worktree_path(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<String>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT conflict_report FROM feature_syncs
                 WHERE feature_id = ?1 AND status = 'conflict'
                 ORDER BY created_at DESC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let res = stmt.query_row(params![feature_id.0], |r| r.get::<_, Option<String>>(0));
        match res {
            Ok(Some(json_str)) => {
                let report: ConflictReport = serde_json::from_str(&json_str)
                    .map_err(|e| format!("Failed to parse conflict report JSON: {}", e))?;
                Ok(report.worktree_path)
            }
            _ => Ok(None),
        }
    }

    fn skip_merge(&self, subtask_run_id: &str, reason: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let now = crate::paths::now_ms();
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
}
