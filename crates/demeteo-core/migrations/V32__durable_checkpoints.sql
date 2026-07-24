-- Durable sequence-step run state (PRD_DAG_WORKFLOWS §5.4, task P1.9).
-- Both tables replace in-memory HashMaps on the execution driver that
-- evaporated on restart, making crash-resume re-run committed work:
--
-- * `sequence_checkpoints` — task ids a sequence step already merged to
--   the feature branch via a mid-list checkpoint. Read by
--   `resolve_task_plan` so the next attempt (in this process or after a
--   restart) runs only the remainder. Cleared when the step completes.
-- * `sequence_plan_cache` — the last *full* task plan a sequence step
--   resolved, stored with the attempt that produced it (V31
--   `step_attempts.attempt_no`), so a targeted retry re-runs only the
--   tasks owning a verdict's implicated files even across a restart.
--
-- Keyed per (feature, node): a workflow may hold several sequence
-- nodes, and handing one node's state to another would run the wrong
-- task list entirely.
CREATE TABLE IF NOT EXISTS sequence_checkpoints (
    feature_id      TEXT NOT NULL,
    step_id         TEXT NOT NULL,
    -- JSON array of landed task ids, in landed order.
    landed_task_ids TEXT NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY (feature_id, step_id)
);

CREATE TABLE IF NOT EXISTS sequence_plan_cache (
    feature_id TEXT NOT NULL,
    step_id    TEXT NOT NULL,
    -- Serialized TaskPlan (the full decomposition, never a targeted
    -- subset — a fragment must not shadow the complete plan).
    plan_json  TEXT NOT NULL,
    -- The step_attempts.attempt_no whose dispatch produced this plan;
    -- NULL when attempt accounting was unavailable.
    attempt_no INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (feature_id, step_id)
);
