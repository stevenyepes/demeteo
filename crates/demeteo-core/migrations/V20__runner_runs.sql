-- Headless-runner run submissions (docs/REMOTE_EXECUTION_PLAN.md M3.2).
-- `run_id` is client-generated (a laptop-side UUID), not server-assigned —
-- that's what makes `submit_run` idempotent: re-submitting the same
-- `run_id` looks up this row instead of creating a duplicate project/
-- feature. Only relevant to `demeteo-runner`'s own database; the Tauri
-- app's database gets this table too (shared migration set) but never
-- populates it.
CREATE TABLE IF NOT EXISTS runner_runs (
    run_id       TEXT PRIMARY KEY,
    project_id   TEXT,
    feature_id   TEXT,
    spec_json    TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending', -- 'pending' | 'running' | 'awaiting_mr' | 'completed' | 'failed'
    error        TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
