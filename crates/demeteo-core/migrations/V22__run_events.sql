-- Append-only per-run event log for the headless runner
-- (docs/REMOTE_EXECUTION_PLAN.md M3.3). `id` (the SQLite rowid) is the
-- monotonic offset `stream_events(run_id, from_offset)` pages against —
-- catching up after a dropped connection is just "give me everything
-- with id > from_offset," no separate sequence counter needed.
CREATE TABLE IF NOT EXISTS run_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id       TEXT NOT NULL,
    kind         TEXT NOT NULL,
    payload_json TEXT,
    created_at   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_run_events_run_id ON run_events(run_id, id);
