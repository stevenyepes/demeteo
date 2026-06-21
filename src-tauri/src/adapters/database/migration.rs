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
         SET agents = '[{\"kind\":\"opencode\",\"enabled\":true},{\"kind\":\"hermes\",\"enabled\":true},{\"kind\":\"claude-code\",\"enabled\":true},{\"kind\":\"antigravity\",\"enabled\":true}]'
         WHERE id = 'local' AND (agents IS NULL OR agents = '' OR agents = '[]');",
        [],
    )?;
    add_column_if_missing(conn, "machines", "auto_approved_rules", "TEXT")?;
    add_column_if_missing(conn, "machines", "use_login_shell", "INTEGER")?;
    add_column_if_missing(conn, "machines", "setup_commands", "TEXT")?;
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
