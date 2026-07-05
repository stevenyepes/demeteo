use super::*;
use crate::adapters::database::{SqliteAdapter, SqliteConnection};

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
             created_at INTEGER NOT NULL,
             completed_at INTEGER,
             merge_commit_sha TEXT,
             conflict_report TEXT,
             resolution_attempts INTEGER NOT NULL DEFAULT 0
         );
         INSERT INTO features (id, project_id, title, status, total_cost, duration, mr_state, created_at)
         VALUES ('f-test', 'p-test', 't', 'running', 0.0, '0s', 'none', 1);
         INSERT INTO subtask_runs (id, feature_id, step_execution_id, subtask_id, worktree_path, branch, started_at)
         VALUES ('sr-x', 'f-test', 'se-x', 'sub-1', '/tmp/r', 'feature/x_subtask_sub-1', 1);"
    ).unwrap();

    let adapter = SqliteAdapter { conn: conn.clone() };
    adapter
        .record_merge_outcome(
            "sr-x",
            &FeatureId::from("f-test".to_string()),
            "feature/x_subtask_sub-1",
            "feature/x",
            "ok",
            Some("commit-sha"),
            None,
            1,
        )
        .unwrap();

    let count: i64 = conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM subtask_merges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn conflict_files_parsed_from_porcelain() {
    let sample = "UU src/foo.rs\nAA new_module.rs\nDU deleted_by_us.txt\n?? untracked\n";
    let lines: Vec<ConflictFile> = sample
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
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
