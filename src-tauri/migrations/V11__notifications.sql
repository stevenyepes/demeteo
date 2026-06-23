-- V11: in-app notification table.
--
-- A `notification` is a user-visible event recorded by the
-- orchestrator for surfacing in the global notification bell. The
-- background MR-state monitor (see `adapters::scheduler::check_mr_states`)
-- is the primary writer for `mr_merged` rows; other variants are
-- produced by their respective adapters (gate step handler, merge
-- executor, etc.) when their event fires.
--
-- The table is intentionally narrow — the `kind` column is an
-- application-level tag, and the UI reads `message` directly without
-- joining back to features. `read` is a tri-state-with-default for
-- "is the user aware of this?" so the bell badge can show an
-- unread count without scanning history.
--
-- Index supports the two queries the UI actually issues:
--   1. `list(project_id?, recent 50)`
--   2. `count_unread()` across all projects
-- `created_at` is unix-ms; using INTEGER (i64) keeps it consistent
-- with the rest of the schema (`features.created_at`, etc.).

CREATE TABLE IF NOT EXISTS notifications (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL,
    feature_id    TEXT NOT NULL,
    kind          TEXT NOT NULL,           -- mr_merged | gate_pending | step_failed | feature_completed | merge_conflict
    message       TEXT NOT NULL,
    feature_url   TEXT,                    -- deep link to /projects/:pid/features/:fid
    read          INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notifications_recent
    ON notifications(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_notifications_unread
    ON notifications(read, created_at DESC)
    WHERE read = 0;

CREATE INDEX IF NOT EXISTS idx_notifications_project
    ON notifications(project_id, created_at DESC);
