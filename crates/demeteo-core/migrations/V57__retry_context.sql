-- The execution driver's retry context, which until now lived only in
-- memory: any restart resumed a step inside a redirect loop as though it
-- were a first pass — greenfield template, no verdict, stale-plan guards
-- off. One row per feature because a driver carries at most one loop.
--
-- A mirror, not an authority: the redirect budget stays on
-- `step_executions.iteration_count`, and a row the restore cannot trust is
-- dropped (`domain::rework::restore_retry_context`).
CREATE TABLE IF NOT EXISTS retry_contexts (
    feature_id            TEXT PRIMARY KEY REFERENCES features(id) ON DELETE CASCADE,
    failing_step_id       TEXT NOT NULL,
    feedback              TEXT NOT NULL,
    iteration             INTEGER NOT NULL,
    max                   INTEGER NOT NULL,
    -- JSON arrays of strings.
    failing_tests_json    TEXT NOT NULL,
    implicated_files_json TEXT NOT NULL,
    updated_at            INTEGER NOT NULL
);
