-- Laptop-side mirror of remote runs submitted to a `demeteo-runner`
-- (docs/REMOTE_EXECUTION_PLAN.md M6.1/M6.2, design R9). Keyed by
-- `(machine_id, run_id)`, not `run_id` alone -- the same laptop-generated
-- `run_id` idempotency key is only unique per machine. Only the Tauri
-- app populates this table; it exists in the runner's own database too
-- (shared migration set) but the runner never writes to it.
CREATE TABLE IF NOT EXISTS remote_run_mirror (
    machine_id           TEXT NOT NULL,
    run_id               TEXT NOT NULL,
    project_id           TEXT,
    title                TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'pending',
    error                TEXT,
    feature_id           TEXT,
    pr_url               TEXT,
    last_offset          INTEGER NOT NULL DEFAULT 0,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    last_notified_status TEXT,
    PRIMARY KEY (machine_id, run_id)
);

CREATE INDEX IF NOT EXISTS idx_remote_run_mirror_updated_at ON remote_run_mirror(updated_at);
