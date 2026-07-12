use refinery::embed_migrations;
use rusqlite::Connection;

use super::error::DbError;

embed_migrations!("migrations");

/// Run all pending refinery migrations, then apply any additional column
/// ALTER TABLE statements that may be needed for databases created before
/// those columns existed in V1.
pub fn run(conn: &mut Connection) -> Result<(), DbError> {
    migrations::runner()
        .set_abort_divergent(false)
        .run(conn)
        .map_err(|e| DbError::Migration(e.to_string()))?;

    add_column_if_missing(conn, "machines", "agents", "TEXT")?;
    conn.execute(
        "UPDATE machines
         SET agents = '[{\"kind\":\"opencode\",\"enabled\":true},{\"kind\":\"hermes\",\"enabled\":true},{\"kind\":\"claude-code\",\"enabled\":true}]'
         WHERE id = 'local' AND (agents IS NULL OR agents = '' OR agents = '[]');",
        [],
    )?;
    // Strip the removed `antigravity` agent from any previously-seeded
    // `agents` JSON so stale rows don't advertise an agent the registry no
    // longer resolves. `get_agent_configs`' parser already drops unsupported
    // kinds on read, but scrubbing the stored value keeps the DB truthful.
    conn.execute(
        "UPDATE machines
         SET agents = replace(replace(replace(agents,
             '{\"kind\":\"antigravity\",\"enabled\":true},', ''),
             ',{\"kind\":\"antigravity\",\"enabled\":true}', ''),
             '{\"kind\":\"antigravity\",\"enabled\":true}', '')
         WHERE agents LIKE '%antigravity%';",
        [],
    )?;
    add_column_if_missing(conn, "machines", "auto_approved_rules", "TEXT")?;
    add_column_if_missing(conn, "machines", "use_login_shell", "INTEGER")?;
    add_column_if_missing(conn, "machines", "setup_commands", "TEXT")?;
    // Optional per-machine "away" notification webhook (docs/
    // REMOTE_EXECUTION_PLAN.md M6.3 follow-up) — makes the channel
    // configurable in-app instead of requiring a shell env var set on
    // the remote host with zero UI discoverability. Injected into the
    // runner's systemd unit environment at install time.
    add_column_if_missing(conn, "machines", "notify_webhook_url", "TEXT")?;
    add_column_if_missing(conn, "thread_sessions", "agent_kind", "TEXT")?;
    add_column_if_missing(conn, "thread_sessions", "updated_at", "INTEGER")?;
    add_column_if_missing(conn, "thread_sessions", "model", "TEXT")?;
    add_column_if_missing(conn, "features", "workflow_id", "TEXT")?;
    add_column_if_missing(conn, "features", "agent_kind", "TEXT")?;
    add_column_if_missing(conn, "features", "model", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "build_command", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "coverage_command", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "conventions_file", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "default_agent_kind", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "default_model", "TEXT")?;
    // Project-level writability exceptions for the chmod scope fence (V18).
    add_column_if_missing(conn, "project_settings", "extra_writable_paths", "TEXT")?;
    // Optional pre-harness prepare command (npm ci, cargo fetch, …) run
    // inside each subtask worktree before the verifier's harness command.
    add_column_if_missing(conn, "project_settings", "prepare_command", "TEXT")?;
    // Per-feature user attachments (V19). Defensive add for pre-existing databases.
    add_column_if_missing(conn, "features", "attachments_json", "TEXT")?;
    // Persisted feature description / prompt body (V27). Defensive for
    // pre-existing databases; previously the description lived only in the
    // agent prompt and was never stored on the row.
    add_column_if_missing(conn, "features", "description", "TEXT NOT NULL DEFAULT ''")?;
    // Memory v2 enrichment (V17) — defensive for pre-existing databases.
    add_column_if_missing(conn, "project_memory", "memory_type", "TEXT")?;
    add_column_if_missing(conn, "project_memory", "statement", "TEXT")?;
    add_column_if_missing(conn, "project_memory", "embedding", "BLOB")?;
    add_column_if_missing(conn, "project_memory", "embedding_model", "TEXT")?;
    add_column_if_missing(conn, "project_memory", "last_used_at", "INTEGER")?;
    add_column_if_missing(
        conn,
        "project_memory",
        "use_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    // Prompt-cache telemetry (V20). Previously computed by the
    // UsageAccumulator and surfaced only on the live `StepProgress`
    // notification — never persisted, so it vanished on the next
    // reload. See adapters/step_executor/updates.rs.
    add_column_if_missing(
        conn,
        "step_executions",
        "cache_read_input_tokens",
        "INTEGER",
    )?;
    add_column_if_missing(
        conn,
        "step_executions",
        "cache_creation_input_tokens",
        "INTEGER",
    )?;
    // Harness-failure triage (C6, docs/EXECUTION_CONSISTENCY_PLAN.md). A
    // normalized fingerprint of the previous attempt's failing harness/prepare
    // output, so the driver can tell a *persistent* (reproduces unchanged)
    // failure from one that is still changing across retries — the signal that
    // gates the regression-vs-environment triage agent.
    add_column_if_missing(conn, "step_executions", "last_failure_fingerprint", "TEXT")?;

    // The PR title/body the `finalize` step's agent wrote. Persisted on the
    // feature rather than passed in memory because the step that authors them
    // and the code that opens the PR are deliberately separated: the headless
    // runner holds no git credential during a run at all (§6.2), so it cannot
    // publish until the very end, long after the finalize step is done.
    add_column_if_missing(conn, "features", "pr_title", "TEXT")?;
    add_column_if_missing(conn, "features", "pr_body", "TEXT")?;

    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    col_type: &str,
) -> Result<(), DbError> {
    let exists: bool = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .any(|name| name == column);

    if !exists {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {col_type}");
        conn.execute(&sql, [])?;
    }
    Ok(())
}
