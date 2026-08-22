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
    // Nothing seeds an agent list for the local host here: it has no `machines`
    // row to seed (V38), and what its default *should* be is decided from
    // what is installed, by `AgentConfig::default_for`.
    //
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
    // REMOTE_EXECUTION.md M6.3 follow-up) — makes the channel
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
    // Harness-failure triage (C6, docs/EXECUTION_PARITY.md). A
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

    // Effort level (V29), the reasoning-budget peer of `model`. Nullable with
    // no default: NULL = inherit, so pre-V29 rows resolve to
    // `EffortLevel::DEFAULT` without a backfill.
    add_column_if_missing(conn, "features", "effort", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "default_effort", "TEXT")?;
    add_column_if_missing(conn, "project_workflow_overrides", "effort", "TEXT")?;

    // Per-turn dollar budget (V30), the `--max-budget-usd` peer of
    // `loop_iterations`. Nullable REAL with no default: NULL = inherit, so
    // pre-V30 rows resolve to the engine default without a backfill.
    add_column_if_missing(conn, "features", "max_budget_usd", "REAL")?;
    add_column_if_missing(conn, "project_settings", "default_max_budget_usd", "REAL")?;

    // The project's chosen default Workflow (V40). Nullable with no default:
    // NULL = unset, so pre-V40 rows keep falling back to the first workflow the
    // list returns without a backfill.
    add_column_if_missing(conn, "project_settings", "default_workflow_id", "TEXT")?;

    // Applied retry-policy rule id (P1.10). Defensive for databases that
    // ran V31 before the column joined the table on the same branch.
    add_column_if_missing(conn, "step_attempts", "applied_rule", "TEXT")?;

    // Workspace fingerprint + idempotency key (P1.14). Same defensive
    // pattern: V31 grew these on the same branch after landing.
    add_column_if_missing(conn, "step_attempts", "workspace_fingerprint", "TEXT")?;
    add_column_if_missing(conn, "step_attempts", "idempotency_key", "TEXT")?;

    // Pinned workflow version (V33, decision 38). Defensive for databases
    // migrated between the V-file landing and this branch merging.
    add_column_if_missing(conn, "features", "workflow_version_id", "TEXT")?;

    // Schema-v2 definition document (V34, P3.6). Same defensive pattern.
    add_column_if_missing(conn, "workflow_versions", "definition_json", "TEXT")?;

    // Where a sequence step's landed prefix is committed (V35). Defensive
    // for databases that ran V32 before the column joined the table.
    add_column_if_missing(conn, "sequence_checkpoints", "anchor_sha", "TEXT")?;

    // What the harnesses said before the feature (V37, decision 44).
    // Nullable, and NULL means *absent*, never "everything was green".
    add_column_if_missing(conn, "features", "harness_baseline_json", "TEXT")?;

    // Where the run started, what its diff is measured against, and what its
    // branch is called (V41). All three nullable, and all three degrade to
    // the pre-V41 behaviour — see the migration's header.
    add_column_if_missing(conn, "features", "origin_json", "TEXT")?;
    add_column_if_missing(conn, "features", "diff_base_branch", "TEXT")?;
    add_column_if_missing(conn, "features", "resolved_branch", "TEXT")?;

    // The project's review entrypoint (V42). Nullable, and NULL reads the same
    // as the empty string — see the migration's header.
    add_column_if_missing(conn, "project_settings", "review_entrypoint", "TEXT")?;

    // The harness/model/effort a project wants its merge conflicts resolved
    // with (V44). All three nullable, and all three inherit — see the
    // migration's header.
    add_column_if_missing(conn, "project_settings", "sync_resolver_agent_kind", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "sync_resolver_model", "TEXT")?;
    add_column_if_missing(conn, "project_settings", "sync_resolver_effort", "TEXT")?;

    // Whether a resolution has reached origin, and whether this project wants
    // one held for review first (V45). Both nullable, and on both NULL is
    // "not yet" / "no opinion" rather than a negative — see the migration's
    // header.
    add_column_if_missing(conn, "sync_sessions", "pushed_at", "INTEGER")?;
    add_column_if_missing(
        conn,
        "project_settings",
        "sync_review_before_push",
        "INTEGER",
    )?;

    // What the user handed the interviewer (V48). Nullable, and NULL reads as
    // the empty list — see the migration's header.
    add_column_if_missing(conn, "discoveries", "attachments_json", "TEXT")?;

    // What an interview turn did, beside what it cost (V49). Nullable, and
    // NULL reads as *absent* rather than as a turn that touched nothing — see
    // the migration's header.
    add_column_if_missing(conn, "discovery_messages", "activity_json", "TEXT")?;

    // The decompose pass nobody has reviewed yet (V50). Nullable, and NULL is
    // the resting state — see the migration's header.
    add_column_if_missing(conn, "discoveries", "pending_proposal_json", "TEXT")?;

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

#[cfg(test)]
#[path = "../../../tests/infrastructure/database/migration_upgrade.rs"]
mod tests;
