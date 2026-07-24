-- Per-attempt history for step executions (PRD_DAG_WORKFLOWS §5.3, task
-- P1.8). Retries stop overwriting history: every dispatch of a step by
-- the driver opens one row here and closes it with the attempt's own
-- outcome, spend, and failure classification — the step_executions row
-- keeps carrying the cumulative totals it always has. The UI's
-- per-attempt drill-down (P2.3) and the declarative retry policy's
-- "already env-retried" derivation (P1.9/P1.10) read this directly.
CREATE TABLE IF NOT EXISTS step_attempts (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    step_execution_id   TEXT NOT NULL,
    -- 1-based, dense per step execution.
    attempt_no          INTEGER NOT NULL,
    -- running | completed | failed | cancelled | interrupted | redirected
    status              TEXT NOT NULL,
    -- This attempt's own spend (deltas), not the step's running total.
    cost_usd            REAL,
    tokens              INTEGER,
    wall_clock_ms       INTEGER,
    -- Failure class per the retry-policy vocabulary:
    -- environment | verdict | agent_failure | non_retryable. NULL for
    -- non-failure outcomes.
    error_class         TEXT,
    -- Normalized failure output (normalize_failure_fingerprint), for
    -- "same failure again?" comparisons across attempts.
    failure_fingerprint TEXT,
    -- The retry-policy rule that answered this failure (P1.10), as
    -- `<class>.<strategy>` (e.g. `verdict.redirect`). NULL for
    -- non-failure outcomes and cancel-preempted failures.
    applied_rule        TEXT,
    started_at          INTEGER NOT NULL,
    ended_at            INTEGER,
    UNIQUE(step_execution_id, attempt_no)
);

CREATE INDEX IF NOT EXISTS idx_step_attempts_step
    ON step_attempts(step_execution_id, attempt_no);
