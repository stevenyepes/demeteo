-- Phase R8: explicit schema version row.
-- The refinery runner tracks its own migration history; this table
-- gives Demeteo a single, app-readable "what version are we on?" row
-- the migration log and the "wipe-and-reinit" flow can read.
-- Inserted as version 1 here; subsequent additive migrations bump it.
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    description TEXT NOT NULL,
    installed_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO schema_version (version, description, installed_at)
VALUES (1, 'V1 initial tables (legacy + redesign)', CAST(strftime('%s', 'now') AS INTEGER) * 1000);
