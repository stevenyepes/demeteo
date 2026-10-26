//! SQL for `hub_consumed_requests`, `hub_request_log` and `hub_local_policy`
//! (V65). The policy's reader and writer are separate traits on purpose; see
//! [`HubPolicyWriter`].

use rusqlite::{params, OptionalExtension};

use crate::domain::hub_request::log::{parse_scope, scope_as_str};
use crate::domain::hub_request::{
    Consumption, HubLocalPolicy, HubRequestLogEntry, LogKind, LogOutcome,
};
use crate::error::AppError;
use crate::ports::hub::{HubPolicyReader, HubPolicyWriter, HubRequestRepository};

use super::super::{DbError, SqliteAdapter};

/// How long past its `expires_at` a consumed id is kept. The Hub's clock has
/// no part in it: an id at or past `expires_at` by this desktop's clock is
/// refused `expired` before it is consumed, whatever the Hub sent it at. The
/// margin is for this desktop's clock stepping backwards after a prune, which
/// would make an id pruned at `expires_at` live again, and fresh.
const PRUNE_SKEW_SECS: u64 = 600;

const LOG_COLUMNS: &str = "request_id, kind, scope, outcome, reason, feature_id, received_at";

const POLICY_COLUMNS: &str = "spend_enabled, gates_enabled, run_ceiling_cents, daily_ceiling_cents";

fn to_sql_int(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unrecognised(col: usize, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        format!("unrecognised value {value:?}").into(),
    )
}

fn row_to_log_entry(row: &rusqlite::Row) -> rusqlite::Result<HubRequestLogEntry> {
    let kind: String = row.get(1)?;
    let scope: Option<String> = row.get(2)?;
    let outcome: String = row.get(3)?;
    Ok(HubRequestLogEntry {
        request_id: row.get(0)?,
        kind: LogKind::parse(&kind).ok_or_else(|| unrecognised(1, kind))?,
        scope: scope
            .map(|s| parse_scope(&s).ok_or_else(|| unrecognised(2, s)))
            .transpose()?,
        outcome: LogOutcome::parse(&outcome).ok_or_else(|| unrecognised(3, outcome))?,
        reason: row.get(4)?,
        feature_id: row.get(5)?,
        received_at_ms: row.get(6)?,
    })
}

/// A ceiling that does not fit a `u64` reads as unset, which refuses every
/// Hub start rather than leaving it unbounded.
fn row_to_policy(row: &rusqlite::Row) -> rusqlite::Result<HubLocalPolicy> {
    let ceiling = |col: usize| -> rusqlite::Result<Option<u64>> {
        Ok(row
            .get::<_, Option<i64>>(col)?
            .and_then(|v| u64::try_from(v).ok()))
    };
    Ok(HubLocalPolicy {
        spend_enabled: row.get::<_, i64>(0)? != 0,
        gates_enabled: row.get::<_, i64>(1)? != 0,
        run_ceiling_cents: ceiling(2)?,
        daily_ceiling_cents: ceiling(3)?,
    })
}

impl HubRequestRepository for SqliteAdapter {
    fn try_consume(
        &self,
        request_id: &str,
        expires_at_secs: u64,
        now_secs: u64,
    ) -> Result<Consumption, AppError> {
        let conn = self.conn.lock()?;
        let inserted = conn
            .execute(
                "INSERT INTO hub_consumed_requests (request_id, expires_at, consumed_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(request_id) DO NOTHING",
                params![
                    request_id,
                    to_sql_int(expires_at_secs),
                    to_sql_int(now_secs)
                ],
            )
            .map_err(DbError::Sqlite)?;
        Ok(if inserted == 0 {
            Consumption::AlreadyConsumed
        } else {
            Consumption::Fresh
        })
    }

    fn prune_consumed(&self, now_secs: u64) -> Result<(), AppError> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM hub_consumed_requests WHERE expires_at < ?1",
            params![to_sql_int(now_secs.saturating_sub(PRUNE_SKEW_SECS))],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }

    fn append_log(&self, entry: &HubRequestLogEntry) -> Result<(), AppError> {
        let conn = self.conn.lock()?;
        conn.execute(
            &format!(
                "INSERT INTO hub_request_log ({LOG_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![
                entry.request_id,
                entry.kind.as_str(),
                entry.scope.map(scope_as_str),
                entry.outcome.as_str(),
                entry.reason,
                entry.feature_id,
                entry.received_at_ms,
            ],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }

    fn list_log(&self, limit: usize) -> Result<Vec<HubRequestLogEntry>, AppError> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {LOG_COLUMNS} FROM hub_request_log
                 ORDER BY received_at DESC, id DESC LIMIT ?1"
            ))
            .map_err(DbError::Sqlite)?;
        let entries = stmt
            .query_map(
                params![i64::try_from(limit).unwrap_or(i64::MAX)],
                row_to_log_entry,
            )
            .map_err(DbError::Sqlite)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(DbError::Sqlite)?;
        Ok(entries)
    }
}

impl HubPolicyReader for SqliteAdapter {
    fn load(&self) -> Result<HubLocalPolicy, AppError> {
        let conn = self.conn.lock()?;
        let policy = conn
            .query_row(
                &format!("SELECT {POLICY_COLUMNS} FROM hub_local_policy WHERE id = 1"),
                [],
                row_to_policy,
            )
            .optional()
            .map_err(DbError::Sqlite)?;
        Ok(policy.unwrap_or_default())
    }
}

impl HubPolicyWriter for SqliteAdapter {
    fn save(&self, policy: &HubLocalPolicy) -> Result<(), AppError> {
        let conn = self.conn.lock()?;
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO hub_local_policy (id, {POLICY_COLUMNS})
                 VALUES (1, ?1, ?2, ?3, ?4)"
            ),
            params![
                policy.spend_enabled,
                policy.gates_enabled,
                policy.run_ceiling_cents.map(to_sql_int),
                policy.daily_ceiling_cents.map(to_sql_int),
            ],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }
}
