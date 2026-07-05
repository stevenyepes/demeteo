-- Memory signals: a lightweight, synchronous queue of "things that happened"
-- during a feature run (gate feedback, failures/retries, agent run summaries).
-- The background memory worker consumes unprocessed rows and distills them into
-- project memories. Capturing a signal is free; analysis is async + LLM-backed.
CREATE TABLE IF NOT EXISTS memory_signals (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL,
    feature_id        TEXT NOT NULL,
    step_execution_id TEXT,
    kind              TEXT NOT NULL,   -- 'agent_summary' | 'failure' | 'retry' | 'gate_feedback'
    content           TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    processed_at      INTEGER,         -- NULL = unprocessed
    attempts          INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ms_unprocessed
    ON memory_signals(processed_at, feature_id);
