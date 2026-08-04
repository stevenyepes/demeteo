-- Worktree directories Demeteo failed to delete, kept until it succeeds
-- or a human deals with them (docs/WINDOWS_PARITY.md, Phase 3).
--
-- Deletion is best-effort on Windows for reasons no retry budget bounds:
-- a scanner, an indexer or an editor can hold a handle open for minutes.
-- The alternative to a durable record is the failure mode every
-- comparable tool's worktree bug report describes — leftover trees
-- accumulating unnoticed until a disk fills. So a failed delete becomes
-- a row, and the row is what the retry sweep and the user-facing notice
-- both read.
--
-- A table rather than a JSON column or a `run_events` append: entries
-- are looked up individually by (machine, path), mutated in place as
-- attempts accrue, and *deleted* when the path finally goes away. An
-- append-only log can only shadow an entry, never retract one, and
-- "which paths are still leaked" would be a whole-log scan and a fold.
--
-- (machine_id, path) is the primary key, not a surrogate id: idempotent
-- enqueue is the point. The same directory failing on three consecutive
-- runs is one problem with three attempts, not three problems. `path` is
-- stored normalized by the port module, so a trailing separator cannot
-- fork one directory into two rows.
--
-- `machine_id` is a bare TEXT with no foreign key, and it holds
-- `LOCAL_MACHINE` for the desktop host, which by policy has no `machines`
-- row at all (V38). A path is only actionable on the machine whose
-- filesystem holds it, so it is half of the identity rather than a
-- decoration.
--
-- `feature_id` likewise carries no foreign key, deliberately. The
-- directory outlives the feature: a user who deletes the feature has not
-- deleted the folder, and an ON DELETE CASCADE would erase the only
-- record that it exists. It is nullable and exists so the notice can say
-- what the leftover was, since the on-disk segment is an 8-hex prefix
-- that identifies nothing to a reader.
--
-- Nothing here expires a row on attempt count. Past the automatic cap an
-- entry stops being retried and starts being reported; it leaves the
-- table only when the directory is confirmed gone.
--
-- `attempts` is cumulative for the life of the row and `auto_attempt_base`
-- is where the automatic budget is measured from, so a user-requested
-- retry grants a fresh budget without rewriting history. One counter
-- doing both jobs would have to be zeroed on reset, and the number the
-- user is shown — how many times this has been tried — would then be
-- false precisely for the entries that have been tried most.
CREATE TABLE IF NOT EXISTS worktree_cleanup_queue (
    machine_id        TEXT NOT NULL,
    path              TEXT NOT NULL,
    feature_id        TEXT,
    last_error        TEXT NOT NULL,
    attempts          INTEGER NOT NULL DEFAULT 1,
    auto_attempt_base INTEGER NOT NULL DEFAULT 0,
    first_enqueued_at INTEGER NOT NULL,
    last_attempt_at   INTEGER NOT NULL,
    PRIMARY KEY (machine_id, path)
);
