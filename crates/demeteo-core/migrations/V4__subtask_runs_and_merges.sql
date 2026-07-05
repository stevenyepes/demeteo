-- Phase R6: per-subtask run + merge audit tables.
--
-- A `subtask_run` is one execution attempt of one subtask spawned by a
-- `parallel` step. The merge that brought it back into the feature
-- branch is recorded in `subtask_merges`. Splitting these from
-- `step_executions` keeps the per-step telemetry clean (it stays
-- step-shaped) while letting the merge audit carry conflict reports,
-- merge SHAs, and resolution attempts that don't fit the step model.
--
-- Both tables default to non-existent on old DBs (CREATE TABLE IF NOT
-- EXISTS) and the new columns on `subtask_merges` are nullable, so
-- re-running this migration on an existing v3 DB is safe.

CREATE TABLE IF NOT EXISTS subtask_runs (
    id              TEXT PRIMARY KEY,
    feature_id      TEXT NOT NULL,
    step_execution_id TEXT NOT NULL,
    subtask_id      TEXT NOT NULL,           -- 'sub-1', 'sub-2', ... from the planner DAG
    agent_id        TEXT,                    -- which agent session ran it (nullable for not-yet-started)
    worktree_path   TEXT NOT NULL,
    branch          TEXT NOT NULL,           -- the subtask branch, e.g. feature/<slug>_subtask_sub-1
    status          TEXT NOT NULL DEFAULT 'pending', -- pending|running|completed|failed|skipped
    cost_usd        REAL NOT NULL DEFAULT 0.0,
    error_message   TEXT,
    started_at      INTEGER NOT NULL,
    ended_at        INTEGER,
    FOREIGN KEY(feature_id) REFERENCES features(id) ON DELETE CASCADE,
    FOREIGN KEY(step_execution_id) REFERENCES step_executions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_subtask_runs_feature
    ON subtask_runs(feature_id, started_at DESC);

CREATE TABLE IF NOT EXISTS subtask_merges (
    id                  TEXT PRIMARY KEY,
    subtask_run_id      TEXT NOT NULL,
    feature_id          TEXT NOT NULL,
    source_branch       TEXT NOT NULL,
    target_branch       TEXT NOT NULL,       -- the feature branch (feature/<slug>)
    status              TEXT NOT NULL DEFAULT 'pending', -- pending|ok|conflict|skipped|aborted
    merge_commit_sha    TEXT,                -- set on status='ok'
    conflict_report     TEXT,                -- JSON-encoded ConflictReport on status='conflict'
    resolution_attempts INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    completed_at        INTEGER,
    FOREIGN KEY(subtask_run_id) REFERENCES subtask_runs(id) ON DELETE CASCADE,
    FOREIGN KEY(feature_id) REFERENCES features(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_subtask_merges_feature
    ON subtask_merges(feature_id, created_at DESC);

-- The 'mr_url' column on `features` records where (if anywhere) the
-- feature branch was published. Nullable, additive.
ALTER TABLE features ADD COLUMN mr_url TEXT;
ALTER TABLE features ADD COLUMN mr_state TEXT NOT NULL DEFAULT 'none';  -- none|draft|open|merged|closed