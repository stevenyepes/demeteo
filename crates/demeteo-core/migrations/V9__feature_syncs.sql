-- Phase R8: feature-branch upstream sync audit table.
--
-- A `feature_sync` row records one attempt to merge
-- `origin/<default_branch>` into the user's feature branch. This is
-- the upstream counterpart of `subtask_merges` (which audits
-- subtask-to-feature merges inside the workflow). Splitting them keeps
-- the two flows' audit trails independent: a sync failure is not the
-- same shape as a subtask merge failure.
--
-- The table is additive (CREATE TABLE IF NOT EXISTS) and the new
-- columns on existing tables are nullable, so re-running this
-- migration on a v8 DB is safe.

CREATE TABLE IF NOT EXISTS feature_syncs (
    id                  TEXT PRIMARY KEY,
    feature_id          TEXT NOT NULL,
    feature_branch      TEXT NOT NULL,        -- e.g. demeteo/features/f-1712345678
    default_branch      TEXT NOT NULL,        -- e.g. main
    status              TEXT NOT NULL DEFAULT 'pending', -- pending|ok|conflict|skipped|aborted
    merge_commit_sha    TEXT,                 -- set on status='ok'
    conflict_report     TEXT,                 -- JSON-encoded ConflictReport on status='conflict'
    resolution_attempts INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    completed_at        INTEGER,
    FOREIGN KEY(feature_id) REFERENCES features(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_feature_syncs_feature
    ON feature_syncs(feature_id, created_at DESC);
