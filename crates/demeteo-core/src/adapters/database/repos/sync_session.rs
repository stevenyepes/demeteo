//! SQL for `sync_sessions` (V43). The contract — one row per feature, a claim
//! the reader must reconcile — is stated once on [`SyncSessionPort`].

use rusqlite::params;

use crate::domain::ids::FeatureId;
use crate::domain::sync_session::SyncSessionStatus;
use crate::ports::sync_session::{SyncSession, SyncSessionPatch, SyncSessionPort};

use super::super::SqliteAdapter;

const COLUMNS: &str = "feature_id, machine_id, repo_dir, feature_branch, base_branch, status,
     worktree_path, head_before, merge_commit_sha, conflict_files, raw_error, attempts,
     created_at, updated_at";

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<SyncSession> {
    let status: String = row.get(5)?;
    let conflict_files: Option<String> = row.get(9)?;
    Ok(SyncSession {
        feature_id: row.get(0)?,
        machine_id: row.get(1)?,
        repo_dir: row.get(2)?,
        feature_branch: row.get(3)?,
        base_branch: row.get(4)?,
        // A status this build does not know is one nobody can act on, and the
        // worktree it named is not ours to keep alive.
        status: SyncSessionStatus::parse(&status).unwrap_or(SyncSessionStatus::Aborted),
        worktree_path: row.get(6)?,
        head_before: row.get(7)?,
        merge_commit_sha: row.get(8)?,
        conflict_files: conflict_files
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default(),
        raw_error: row.get(10)?,
        attempts: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

impl SyncSessionPort for SqliteAdapter {
    fn open(&self, session: &SyncSession) -> Result<(), String> {
        // The column's NULL means "no conflict has been measured yet", which is
        // what a session opened before the merge is in — distinct from the `[]`
        // a porcelain read writes when it answered nothing. Serializing an empty
        // Vec into `[]` here would erase that distinction on every open.
        let files = if session.conflict_files.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&session.conflict_files).map_err(|e| e.to_string())?)
        };
        let conn = self.conn.lock()?;
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO sync_sessions ({COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
            ),
            params![
                session.feature_id,
                session.machine_id,
                session.repo_dir,
                session.feature_branch,
                session.base_branch,
                session.status.as_str(),
                session.worktree_path,
                session.head_before,
                session.merge_commit_sha,
                files,
                session.raw_error,
                session.attempts,
                session.created_at,
                session.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get(&self, feature_id: &FeatureId) -> Result<Option<SyncSession>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM sync_sessions WHERE feature_id = ?1"),
            params![feature_id.0],
            row_to_session,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e.to_string()),
        })
    }

    fn update(
        &self,
        feature_id: &FeatureId,
        patch: &SyncSessionPatch,
        now: i64,
    ) -> Result<(), String> {
        let files = match patch.conflict_files.as_ref() {
            Some(files) => Some(serde_json::to_string(files).map_err(|e| e.to_string())?),
            None => None,
        };
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE sync_sessions
                SET status           = COALESCE(?2, status),
                    worktree_path    = CASE WHEN ?3 THEN ?4 ELSE worktree_path END,
                    head_before      = CASE WHEN ?5 THEN ?6 ELSE head_before END,
                    merge_commit_sha = CASE WHEN ?7 THEN ?8 ELSE merge_commit_sha END,
                    conflict_files   = COALESCE(?9, conflict_files),
                    raw_error        = CASE WHEN ?10 THEN ?11 ELSE raw_error END,
                    attempts         = attempts + ?12,
                    updated_at       = ?13
              WHERE feature_id = ?1",
            params![
                feature_id.0,
                patch.status.map(|s| s.as_str()),
                patch.worktree_path.is_some(),
                patch.worktree_path.clone().flatten(),
                patch.head_before.is_some(),
                patch.head_before.clone().flatten(),
                patch.merge_commit_sha.is_some(),
                patch.merge_commit_sha.clone().flatten(),
                files,
                patch.raw_error.is_some(),
                patch.raw_error.clone().flatten(),
                i64::from(patch.bump_attempts),
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn close(&self, feature_id: &FeatureId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM sync_sessions WHERE feature_id = ?1",
            params![feature_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/sync_session.rs"]
mod tests;
