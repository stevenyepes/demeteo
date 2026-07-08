use rusqlite::params;

use crate::ports::run_events::{RunEvent, RunEventsPort};
use crate::shared::secret_scrub::scrub_secrets;

use super::super::SqliteAdapter;

fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<RunEvent> {
    Ok(RunEvent {
        offset: row.get(0)?,
        run_id: row.get(1)?,
        kind: row.get(2)?,
        payload_json: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl RunEventsPort for SqliteAdapter {
    fn append(
        &self,
        run_id: &str,
        kind: &str,
        payload_json: Option<&str>,
        now: i64,
    ) -> Result<i64, String> {
        // Secret scrubbing (M7.2, §6): the event log is append-only and
        // streamed verbatim to the laptop over the control channel, so it's
        // exactly the sink a credential-bearing foreign error string could
        // leak through. Scrub at the write — the single choke point that
        // covers both `run::emit` and the direct `rpc.rs` failure-path
        // appends — so a missed upstream redaction can't persist a token.
        let payload_json = payload_json.map(|p| scrub_secrets(p).into_owned());
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO run_events (run_id, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![run_id, kind, payload_json, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    fn list_since(&self, run_id: &str, from_offset: i64) -> Result<Vec<RunEvent>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, run_id, kind, payload_json, created_at
                 FROM run_events
                 WHERE run_id = ?1 AND id > ?2
                 ORDER BY id ASC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![run_id, from_offset], row_to_event)
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::database::SqliteAdapter;
    use rusqlite::Connection;

    #[test]
    fn append_scrubs_secrets_from_payload() {
        // The security-relevant invariant (M7.2, §6): a credential that
        // slips into an event payload upstream never reaches the
        // append-only, laptop-streamed log verbatim.
        let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
        let leaky = "\"clone failed: https://x-access-token:ghp_0123456789abcdef0123456789abcdefABCD@github.com/o/r.git\"";
        adapter.append("run-1", "failed", Some(leaky), 1).unwrap();

        let events = adapter.list_since("run-1", 0).unwrap();
        let stored = events[0].payload_json.as_deref().unwrap();
        assert!(
            !stored.contains("ghp_0123456789abcdef"),
            "token leaked: {stored}"
        );
        assert!(stored.contains("***"));
    }
}
